//! Bounded, private identity probes. Probe failures never carry child output.
use super::{Error, Result};
use std::cell::Cell;
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const STANDALONE_TIMEOUT: Duration = Duration::from_secs(30);
const CAPTURE_LIMIT: usize = 8 * 1024 * 1024;
thread_local! { static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) }; }

// Process deadlines are an I/O boundary, outside the deterministic render path.
#[allow(clippy::disallowed_methods)]
fn now() -> Instant {
    Instant::now()
}

pub fn with_deadline<T>(deadline: Option<Instant>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<Instant>);
    impl Drop for Restore {
        fn drop(&mut self) {
            DEADLINE.with(|d| d.set(self.0));
        }
    }
    let deadline = deadline
        .or_else(|| DEADLINE.with(|d| d.get()))
        .unwrap_or_else(|| now() + STANDALONE_TIMEOUT);
    let previous = DEADLINE.with(|d| {
        let old = d.get();
        d.set(Some(old.map_or(deadline, |old| old.min(deadline))));
        old
    });
    let _restore = Restore(previous);
    run()
}

pub fn run(command: &mut Command) -> Result<Output> {
    let deadline = DEADLINE
        .with(|d| d.get())
        .unwrap_or_else(|| now() + STANDALONE_TIMEOUT);
    if now() >= deadline {
        return Err(Error::new("identity_probe_timeout"));
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| Error::new("identity_probe_unavailable"))?;
    let pid = child.id();
    let (send, receive) = mpsc::channel();
    let readers: Vec<_> = [
        (
            0,
            Box::new(child.stdout.take().expect("piped stdout")) as Box<dyn Read + Send>,
        ),
        (
            1,
            Box::new(child.stderr.take().expect("piped stderr")) as Box<dyn Read + Send>,
        ),
    ]
    .into_iter()
    .map(|(stream, reader)| {
        let send = send.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .take((CAPTURE_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
                .map_err(|_| Error::new("identity_probe_unavailable"));
            let _ = send.send((stream, result));
        })
    })
    .collect();
    drop(send);
    let mut streams: [Option<Vec<u8>>; 2] = [None, None];
    let mut status = None;
    let result = loop {
        let mut failure = None;
        for (stream, result) in receive.try_iter() {
            match result {
                Ok(bytes) if bytes.len() <= CAPTURE_LIMIT => streams[stream] = Some(bytes),
                Ok(_) => failure = Some(Error::new("identity_probe_output_limit")),
                Err(e) => failure = Some(e),
            }
        }
        if let Some(error) = failure {
            break Err(error);
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(_) => break Err(Error::new("identity_probe_unavailable")),
            }
        }
        if now() >= deadline {
            break Err(Error::new("identity_probe_timeout"));
        }
        if let Some(status) = status
            && streams.iter().all(Option::is_some)
        {
            break Ok(Output {
                status,
                stdout: streams[0].take().unwrap(),
                stderr: streams[1].take().unwrap(),
            });
        }
        // Keep the deadline active even after the leader exits: descendants can
        // retain the capture pipes, so joining readers early could wait forever.
        std::thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(now())));
    };
    if result.is_err() {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    for reader in readers {
        let _ = reader.join();
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn identity_probe_reaps_the_leader_and_kills_descendants_holding_capture_pipes() {
        let home = tempfile::tempdir().unwrap();
        let pid_file = home.path().join("leader");
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "printf '%s' \"$$\" > \"$1\"; sleep 5 & exit 0",
                "probe",
            ])
            .arg(&pid_file);
        let started = now();
        let result = with_deadline(Some(started + Duration::from_millis(100)), || {
            run(&mut command)
        });
        assert_eq!(result.unwrap_err().code, "identity_probe_timeout");
        assert!(started.elapsed() < Duration::from_secs(1));
        let pid: libc::pid_t = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
        // The probe must have reaped its leader, even when it exited before EOF.
        assert_eq!(
            unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }

    #[test]
    fn identity_probe_capture_limit_refuses_without_returning_child_output() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf 'FIXTURE_CAPTURE_CANARY' >&2; head -c 8388609 /dev/zero",
        ]);
        let error = run(&mut command).unwrap_err();
        assert_eq!(error.code, "identity_probe_output_limit");
        assert!(!format!("{error:?}").contains("FIXTURE_CAPTURE_CANARY"));
    }

    #[test]
    fn identity_probe_nested_preparation_preserves_one_shared_deadline() {
        let started = now();
        let result = with_deadline(Some(started + Duration::from_millis(150)), || {
            let mut first = Command::new("sh");
            first.args(["-c", "sleep 0.1"]);
            run(&mut first)?;
            with_deadline(None, || {
                let mut second = Command::new("sh");
                second.args(["-c", "sleep 0.1"]);
                run(&mut second)
            })
        });
        assert_eq!(result.unwrap_err().code, "identity_probe_timeout");
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
