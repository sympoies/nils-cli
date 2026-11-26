use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use nils_common::{cli_contract::exit, shell::quote_posix_single as quote};

use crate::{Failure, Remote, Work, interrupt::Interrupts};

const SSH_UNAVAILABLE: Failure = Failure::new(
    exit::UNAVAILABLE,
    "ssh-unavailable",
    "Could not start OpenSSH",
);
const INPUT_FILE: Failure = Failure::new(
    exit::DATA,
    "input-file",
    "Could not open the input as a regular file",
);
const LOCAL_IO: Failure = Failure::new(
    exit::RUNTIME,
    "local-io",
    "Could not capture the OpenSSH result",
);

#[derive(Clone, Copy)]
pub(super) enum EndReason {
    Completed,
    TimedOut,
    Interrupted(i32),
}

pub(super) struct Capture {
    pub status: ExitStatus,
    pub end_reason: EndReason,
    pub remote_255: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub started_at: Timestamp,
    pub elapsed_ms: u128,
}

impl Capture {
    pub fn signal(&self) -> Option<i32> {
        self.status.signal()
    }
}

pub(super) fn run(remote: &Remote, work: &Work<'_>) -> Result<Capture, Failure> {
    let stdin = work.input().map(open_input).transpose()?;
    // OpenSSH reserves 255 for transport errors. A per-run marker lets us
    // identify a remote command that actually returned that same status.
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| LOCAL_IO)?;
    let nonce = u128::from_ne_bytes(random);
    let marker = format!("remote-cli-exit-255-{nonce:032x}");
    let command = remote_command(remote, work, &marker);
    // File-backed capture preserves both streams without waiting for EOF
    // from a proxy or other helper that outlives the local OpenSSH process.
    let stdout = tempfile::tempfile().map_err(|_| LOCAL_IO)?;
    let stderr = tempfile::tempfile().map_err(|_| LOCAL_IO)?;
    let interrupts = Interrupts::install().map_err(|_| LOCAL_IO)?;
    let started_at = Timestamp::now();
    let started = Instant::now();
    let mut child = Command::new("ssh")
        .args(["-T", "-o", "BatchMode=yes", "-o"])
        .arg(format!("ConnectTimeout={}", remote.connect_timeout))
        .arg("-o")
        .arg(if stdin.is_some() {
            "StdinNull=no"
        } else {
            "StdinNull=yes"
        })
        .args(["--", &remote.host, &command])
        .stdin(stdin.map_or_else(Stdio::null, Stdio::from))
        .stdout(stdout.try_clone().map_err(|_| LOCAL_IO)?)
        .stderr(stderr.try_clone().map_err(|_| LOCAL_IO)?)
        .process_group(0)
        .spawn()
        .map_err(|_| SSH_UNAVAILABLE)?;

    let (status, end_reason) = wait(
        &mut child,
        started,
        Duration::from_secs(remote.timeout.into()),
        &interrupts,
    )
    .map_err(|_| LOCAL_IO)?;
    let elapsed_ms = started.elapsed().as_millis();
    let stdout = captured_bytes(&stdout).map_err(|_| LOCAL_IO)?;
    let mut stderr = captured_bytes(&stderr).map_err(|_| LOCAL_IO)?;
    let remote_255 = remove_marker(&mut stderr, &marker);
    Ok(Capture {
        status,
        end_reason,
        remote_255,
        stdout,
        stderr,
        started_at,
        elapsed_ms,
    })
}

fn open_input(path: &Path) -> Result<File, Failure> {
    // Opening a FIFO must not block before the deadline handling starts.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| INPUT_FILE)?;
    if !file.metadata().map_err(|_| INPUT_FILE)?.is_file() {
        return Err(INPUT_FILE);
    }
    Ok(file)
}

fn captured_bytes(file: &File) -> io::Result<Vec<u8>> {
    let len = usize::try_from(file.metadata()?.len()).map_err(io::Error::other)?;
    let mut output = vec![0; len];
    file.read_exact_at(&mut output, 0)?;
    Ok(output)
}

fn wait(
    child: &mut Child,
    started: Instant,
    timeout: Duration,
    interrupts: &Interrupts,
) -> io::Result<(ExitStatus, EndReason)> {
    let interval = Duration::from_millis(10);
    loop {
        if let Some(signal) = interrupts.received() {
            return stop(child).map(|status| (status, EndReason::Interrupted(signal)));
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok((status, EndReason::Completed)),
            Ok(None) => {}
            Err(error) => {
                let _ = stop(child);
                return Err(error);
            }
        }
        let pause = if timeout.is_zero() {
            interval
        } else {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return stop(child).map(|status| (status, EndReason::TimedOut));
            }
            remaining.min(interval)
        };
        thread::sleep(pause);
    }
}

fn stop(child: &mut Child) -> io::Result<ExitStatus> {
    // SAFETY: The unreaped child was spawned as the leader of this process
    // group, so sending the signal to its group also stops its local proxies.
    if unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) } != 0 {
        child.kill()?;
    }
    child.wait()
}

fn remote_command(remote: &Remote, work: &Work<'_>, marker: &str) -> String {
    let cwd = match remote.cwd.as_deref() {
        None => String::from("\"$HOME\""),
        Some(path) => remote_directory(path),
    };
    let shell = remote
        .shell
        .as_deref()
        .map_or_else(|| String::from("\"${SHELL:-/bin/sh}\""), quote);
    let (argv, sudo_user) = match work {
        Work::Exec {
            argv, sudo_user, ..
        } => (
            argv.iter()
                .map(|arg| quote(arg))
                .collect::<Vec<_>>()
                .join(" "),
            *sudo_user,
        ),
        Work::Shell { .. } => (format!("{shell} -s"), None),
    };
    let mut invoke = String::from("command -- ");
    if remote.sudo {
        invoke.push_str("sudo -n ");
        if let Some(user) = sudo_user {
            invoke.push_str(&format!("-u {} ", quote(user)));
        }
        invoke.push_str("-- ");
    }
    invoke.push_str(&argv);

    // The login shell loads its profile, then hands off fixed POSIX syntax to
    // /bin/sh. No user-provided argument is substituted into that syntax.
    let run = format!(
        "cd -- {cwd} >/dev/null || exit $?; \
         if ({invoke}); then exit 0; else status=$?; fi; \
         if [ \"$status\" -eq 255 ]; then printf '\\n%s\\n' '{marker}' >&2; fi; \
         exit \"$status\""
    );
    let bootstrap = format!("exec /bin/sh -c {}", quote(&run));
    format!("exec {shell} -lc {}", quote(&bootstrap))
}

fn remote_directory(path: &str) -> String {
    if let Some(rest) = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix("$HOME/"))
    {
        format!("\"$HOME\"/{}", quote(rest))
    } else if path.starts_with('/') || path.is_empty() {
        quote(path)
    } else {
        // Bypass CDPATH, option parsing, and cd's special "-" operand.
        format!("./{}", quote(path))
    }
}

fn remove_marker(stderr: &mut Vec<u8>, marker: &str) -> bool {
    let marker = format!("\n{marker}\n");
    if let Some(start) = stderr
        .windows(marker.len())
        .position(|bytes| bytes == marker.as_bytes())
    {
        stderr.drain(start..start + marker.len());
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::remove_marker;
    use pretty_assertions::assert_eq;

    #[test]
    fn exit_marker_preserves_other_stderr_bytes() {
        let mut stderr = b"prefix\xff\nremote-cli-exit-255-other\n\nremote-cli-exit-255-fixture\nssh diagnostic\n".to_vec();

        assert!(remove_marker(&mut stderr, "remote-cli-exit-255-fixture"));
        assert_eq!(
            stderr,
            b"prefix\xff\nremote-cli-exit-255-other\nssh diagnostic\n"
        );
        assert!(!remove_marker(&mut stderr, "remote-cli-exit-255-fixture"));
    }
}
