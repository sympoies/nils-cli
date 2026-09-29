//! Bounded host account-broker process runner shared by provider bindings.
//!
//! A broker is configured as a JSON argv array, never a shell command. Each
//! invocation runs in a fresh process group with a null stdin, bounded
//! stdout/stderr, and a hard deadline. Callers map [`BrokerProcessError`] to
//! their own stable, provider-specific error codes.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

pub(crate) const BROKER_OUTPUT_LIMIT: u64 = 1024 * 1024;
const MAX_BROKER_ARGV: usize = 16;
const MAX_BROKER_ARG_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrokerArgvError {
    NotJsonArgv,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrokerProcessError {
    SpawnFailed,
    StdoutUnavailable,
    StderrUnavailable,
    WaitFailed,
    Rejected,
    OutputTooLarge,
    MalformedJson,
    Timeout,
}

/// Parses a broker argv configuration value. Blank values mean "not configured".
pub(crate) fn parse_argv(raw: Option<String>) -> Result<Option<Vec<String>>, BrokerArgvError> {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    let argv: Vec<String> = serde_json::from_str(&raw).map_err(|_| BrokerArgvError::NotJsonArgv)?;
    if !valid_argv(&argv) {
        return Err(BrokerArgvError::Invalid);
    }
    Ok(Some(argv))
}

/// Count and per-argument bounds shared by every broker configuration source.
pub(crate) fn valid_argv(argv: &[String]) -> bool {
    !argv.is_empty()
        && argv.len() <= MAX_BROKER_ARGV
        && argv
            .iter()
            .all(|arg| !arg.is_empty() && arg.len() <= MAX_BROKER_ARG_BYTES && !arg.contains('\0'))
}

/// Runs `argv + args` and decodes its stdout as one JSON document.
pub(crate) fn run(
    argv: &[String],
    args: &[&str],
    timeout: Duration,
) -> Result<Value, BrokerProcessError> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|_| BrokerProcessError::SpawnFailed)?;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or(BrokerProcessError::StdoutUnavailable)?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or(BrokerProcessError::StderrUnavailable)?;
    let (output_tx, output_rx) = std::sync::mpsc::channel();
    let stdout_tx = output_tx.clone();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout_pipe
            .by_ref()
            .take(BROKER_OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes);
        let _ = stdout_tx.send((true, bytes));
    });
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr_pipe
            .by_ref()
            .take(BROKER_OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes);
        let _ = output_tx.send((false, bytes));
    });

    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut stdout = None;
    let mut stderr_drained = false;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(exit)) => status = Some(exit),
                Ok(None) => {}
                Err(_) => {
                    terminate(&mut child);
                    return Err(BrokerProcessError::WaitFailed);
                }
            }
        }
        while let Ok((is_stdout, bytes)) = output_rx.try_recv() {
            if is_stdout {
                stdout = Some(bytes);
            } else {
                stderr_drained = true;
            }
        }
        if let (Some(status), Some(stdout)) = (status.as_ref(), stdout.as_ref())
            && stderr_drained
        {
            if !status.success() {
                terminate(&mut child);
                return Err(BrokerProcessError::Rejected);
            }
            if stdout.len() as u64 > BROKER_OUTPUT_LIMIT {
                return Err(BrokerProcessError::OutputTooLarge);
            }
            let decoded =
                serde_json::from_slice(stdout).map_err(|_| BrokerProcessError::MalformedJson);
            terminate(&mut child);
            return decoded;
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            return Err(BrokerProcessError::Timeout);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate(child: &mut std::process::Child) {
    let pid = child.id();
    // SAFETY: the broker is launched as the leader of a fresh process group.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
