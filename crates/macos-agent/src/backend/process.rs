use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::CliError;

#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
}

impl ProcessRequest {
    pub fn new(program: impl Into<String>, args: Vec<String>, timeout_ms: u64) -> Self {
        Self {
            program: program.into(),
            args,
            timeout_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessFailure {
    NotFound {
        program: String,
    },
    Timeout {
        program: String,
        timeout_ms: u64,
    },
    NonZero {
        program: String,
        code: i32,
        stderr: String,
    },
    Io {
        program: String,
        message: String,
    },
}

pub trait ProcessRunner {
    fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessFailure>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RealProcessRunner;

impl ProcessRunner for RealProcessRunner {
    fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessFailure> {
        let mut cmd = Command::new(&request.program);
        cmd.args(&request.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|err| map_spawn_error(&request.program, err))?;

        let deadline = Instant::now() + Duration::from_millis(request.timeout_ms.max(1));
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    let output = child.wait_with_output().map_err(|err| ProcessFailure::Io {
                        program: request.program.clone(),
                        message: err.to_string(),
                    })?;
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    if output.status.success() {
                        return Ok(ProcessOutput { stdout, stderr });
                    }
                    let code = output.status.code().unwrap_or(-1);
                    return Err(ProcessFailure::NonZero {
                        program: request.program.clone(),
                        code,
                        stderr: sanitize_stderr(&stderr),
                    });
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(ProcessFailure::Timeout {
                            program: request.program.clone(),
                            timeout_ms: request.timeout_ms,
                        });
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    return Err(ProcessFailure::Io {
                        program: request.program.clone(),
                        message: err.to_string(),
                    });
                }
            }
        }
    }
}

fn map_spawn_error(program: &str, err: io::Error) -> ProcessFailure {
    if err.kind() == io::ErrorKind::NotFound {
        ProcessFailure::NotFound {
            program: program.to_string(),
        }
    } else {
        ProcessFailure::Io {
            program: program.to_string(),
            message: err.to_string(),
        }
    }
}

fn sanitize_stderr(stderr: &str) -> String {
    let line = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        "no stderr output".to_string()
    } else {
        line
    }
}

pub fn map_failure(operation: &str, failure: ProcessFailure) -> CliError {
    match failure {
        ProcessFailure::NotFound { program } => CliError::runtime(format!(
            "{operation} failed: missing dependency `{program}` in PATH"
        )),
        ProcessFailure::Timeout {
            program,
            timeout_ms,
        } => CliError::timeout(&format!("{operation} via `{program}`"), timeout_ms),
        ProcessFailure::NonZero {
            program,
            code,
            stderr,
        } => CliError::runtime(format!(
            "{operation} failed via `{program}` (exit {code}): {stderr}"
        )),
        ProcessFailure::Io { program, message } => {
            CliError::runtime(format!("{operation} failed to run `{program}`: {message}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{map_failure, ProcessFailure, ProcessRequest, ProcessRunner, RealProcessRunner};

    #[test]
    fn reports_not_found() {
        let runner = RealProcessRunner;
        let req = ProcessRequest::new("__missing_binary__", Vec::new(), 100);
        let err = runner.run(&req).expect_err("missing bin should fail");
        assert_eq!(
            err,
            ProcessFailure::NotFound {
                program: "__missing_binary__".to_string(),
            }
        );
    }

    #[test]
    fn maps_timeout_failure_to_runtime_error() {
        let err = map_failure(
            "test-op",
            ProcessFailure::Timeout {
                program: "osascript".to_string(),
                timeout_ms: 10,
            },
        );

        assert_eq!(err.exit_code(), 1);
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn non_zero_stderr_is_compacted() {
        let runner = RealProcessRunner;
        let req = ProcessRequest::new(
            "sh",
            vec![
                "-c".to_string(),
                "echo 'bad\\nline' 1>&2; exit 3".to_string(),
            ],
            200,
        );
        let err = runner.run(&req).expect_err("script should fail");
        match err {
            ProcessFailure::NonZero { code, stderr, .. } => {
                assert_eq!(code, 3);
                assert_eq!(stderr, "bad line");
            }
            other => panic!("unexpected failure: {other:?}"),
        }
    }
}
