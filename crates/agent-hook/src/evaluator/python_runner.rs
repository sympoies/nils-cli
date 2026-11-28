//! One framed conversation with the installed runtime-kit runner.

use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::HookError;
use crate::model::Product;

use super::runtime_handlers::validate_handler;
use super::terminate_process_group;

const RUNNER_FILENAME: &str = "event-runner.py";
const REPLY_HEADER_LIMIT: u64 = 16;

#[derive(Default)]
pub(super) struct PythonRunner(Option<Runner>);

impl PythonRunner {
    pub(super) fn request(
        &mut self,
        root: &Path,
        product: Product,
        payload: &[u8],
        filename: &str,
        timeout: Duration,
        output_limit: usize,
    ) -> Result<(bool, Vec<u8>), HookError> {
        let deadline = Instant::now() + timeout;
        let needs_payload = self.0.is_none();
        if needs_payload {
            self.0 = Some(Runner::start(root, product)?);
        }
        let runner = self.0.as_mut().expect("runner started");
        let reply = runner.exchange(
            needs_payload.then_some(payload),
            filename,
            output_limit,
            deadline,
        );
        reply.map_err(|error| {
            // Discard the conversation on transport failure. The caller applies
            // this rule's posture and may ask for a different handler next.
            self.0 = None;
            if error.kind() == io::ErrorKind::TimedOut {
                HookError::runtime(
                    "capability-timeout",
                    "capability exceeded its fixed timeout",
                )
            } else {
                HookError::runtime(
                    "handler-failed",
                    "runtime-kit runner did not return a valid reply",
                )
            }
        })
    }
}

pub(super) fn validate_runner(root: &Path) -> Result<PathBuf, HookError> {
    let metadata = fs::symlink_metadata(root).map_err(|_| {
        HookError::runtime(
            "handler-unavailable",
            "runtime-kit hook directory is unavailable",
        )
    })?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(HookError::data(
            "handler-untrusted",
            "runtime-kit hook directory type, owner, or mode is untrusted",
        ));
    }
    let path = root.join(RUNNER_FILENAME);
    validate_handler(&path)?;
    Ok(path)
}

struct Runner(Child);

impl Runner {
    fn start(root: &Path, product: Product) -> Result<Self, HookError> {
        let path = validate_runner(root)?;
        let runner = Self(
            Command::new(path)
                .env("AGENT_RUNTIME_PRODUCT", product.as_str())
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| {
                    HookError::runtime(
                        "capability-unavailable",
                        "runtime-kit runner could not start",
                    )
                })?,
        );
        for fd in [
            runner.0.stdin.as_ref().expect("piped stdin").as_raw_fd(),
            runner.0.stdout.as_ref().expect("piped stdout").as_raw_fd(),
        ] {
            // Keep pipe I/O inside the deadline even when the runner stops
            // reading the initial payload or sends a partial reply.
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags == -1
                || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
            {
                return Err(HookError::runtime(
                    "capability-unavailable",
                    "runtime-kit runner pipes could not be configured",
                ));
            }
        }
        Ok(runner)
    }

    fn exchange(
        &mut self,
        payload: Option<&[u8]>,
        filename: &str,
        output_limit: usize,
        deadline: Instant,
    ) -> io::Result<(bool, Vec<u8>)> {
        let stdin = self.0.stdin.as_mut().expect("piped stdin");
        let mut input = DeadlinePipe {
            pipe: stdin,
            deadline,
        };
        if let Some(payload) = payload {
            writeln!(input, "{}", payload.len())?;
            input.write_all(payload)?;
        }
        writeln!(input, "{output_limit} {filename}")?;

        let stdout = self.0.stdout.as_mut().expect("piped stdout");
        let mut output = BufReader::new(DeadlinePipe {
            pipe: stdout,
            deadline,
        });
        let mut header = Vec::new();
        (&mut output)
            .take(REPLY_HEADER_LIMIT)
            .read_until(b'\n', &mut header)?;
        let (status, length) = std::str::from_utf8(&header)
            .ok()
            .and_then(|header| header.strip_suffix('\n'))
            .and_then(|header| header.split_once(' '))
            .and_then(|(status, length)| Some((decimal::<u8>(status)?, decimal::<usize>(length)?)))
            .filter(|(_, length)| *length <= output_limit + 1)
            .ok_or(io::ErrorKind::InvalidData)?;
        let mut stdout = vec![0; length];
        output.read_exact(&mut stdout)?;
        if !output.buffer().is_empty() {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok((status == 0, stdout))
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        terminate_process_group(&mut self.0);
    }
}

fn decimal<T: std::str::FromStr>(field: &str) -> Option<T> {
    if field.bytes().all(|byte| byte.is_ascii_digit()) {
        field.parse().ok()
    } else {
        None
    }
}

struct DeadlinePipe<'a, T> {
    pipe: &'a mut T,
    deadline: Instant,
}

impl<T: AsRawFd> DeadlinePipe<'_, T> {
    fn io<R>(
        &mut self,
        events: libc::c_short,
        mut op: impl FnMut(&mut T) -> io::Result<R>,
    ) -> io::Result<R> {
        let mut descriptor = libc::pollfd {
            fd: self.pipe.as_raw_fd(),
            events,
            revents: 0,
        };
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::ErrorKind::TimedOut.into());
            }
            match op(self.pipe) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                result => return result,
            }
            let millis = remaining.as_millis().max(1) as libc::c_int;
            if unsafe { libc::poll(&mut descriptor, 1, millis) } < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
}

impl<T: AsRawFd + Read> Read for DeadlinePipe<'_, T> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.io(libc::POLLIN, |pipe| pipe.read(buffer))
    }
}

impl<T: AsRawFd + Write> Write for DeadlinePipe<'_, T> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.io(libc::POLLOUT, |pipe| pipe.write(buffer))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.pipe.flush()
    }
}
