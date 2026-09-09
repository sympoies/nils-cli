use anyhow::{Context, Result};
use std::process::{Command, Stdio};

pub fn run_lines(input: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<(i32, Vec<String>)> {
    let mut cmd = Command::new("fzf");
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in envs {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().context("spawn fzf")?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        // fzf stops reading as soon as it has a selection, so a closed pipe
        // means it no longer needs the rest of the candidates rather than that
        // the command failed. Its own exit status is the verdict.
        match stdin.write_all(input.as_bytes()) {
            Ok(()) => {}
            Err(error) if is_consumer_closed(&error) => {}
            Err(error) => return Err(error).context("write fzf stdin"),
        }
    }

    let output = child.wait_with_output().context("wait fzf")?;
    let code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let lines = stdout.lines().map(|s| s.to_string()).collect();
    Ok((code, lines))
}

/// Whether a write failed because the reader stopped reading.
///
/// Every other failure stays an error, so tolerating this one cannot swallow a
/// genuinely truncated write.
fn is_consumer_closed(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe
}

pub fn run_print_query(
    input: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<(i32, Option<String>, Option<String>)> {
    let (code, lines) = run_lines(input, args, envs)?;
    let query = lines.first().cloned();
    let selected = lines.get(1).cloned();
    Ok((code, query, selected))
}

pub fn run_expect(
    input: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> Result<(i32, Option<String>, Vec<String>)> {
    let (code, lines) = run_lines(input, args, envs)?;
    let key = lines.first().cloned();
    let rest = lines.into_iter().skip(1).collect::<Vec<_>>();
    Ok((code, key, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub that reads one line, answers, and exits without draining stdin,
    /// which is what fzf itself does whenever the user picks early or the
    /// selection is forced.
    fn early_exit_stub() -> nils_test_support::StubBinDir {
        let stub = nils_test_support::StubBinDir::new();
        stub.write_exe(
            "fzf",
            r#"#!/bin/bash
IFS= read -r first || true
printf '%s\n' "$first"
exit 0
"#,
        );
        stub
    }

    fn with_stub_path<T>(stub: &nils_test_support::StubBinDir, body: impl FnOnce() -> T) -> T {
        let lock = nils_test_support::GlobalStateLock::new();
        let _path = nils_test_support::prepend_path(&lock, stub.path());
        body()
    }

    /// fzf exiting before it has consumed every candidate is ordinary, not a
    /// failure, so the write that lands on the closed pipe must not abort the
    /// command.
    ///
    /// A large input makes the pipe buffer overflow deterministically, so the
    /// write cannot complete before the stub exits. Under load the same
    /// condition appeared as `write fzf stdin: Broken pipe (os error 32)` and
    /// failed several `fzf-cli` integration tests.
    #[test]
    fn an_early_exiting_fzf_does_not_fail_the_write() {
        let stub = early_exit_stub();
        let mut input = String::from("first\n");
        for index in 0..200_000 {
            input.push_str(&format!("candidate-{index}\n"));
        }

        let result = with_stub_path(&stub, || run_lines(&input, &[], &[]));

        let (code, lines) = result.expect("an early exiting fzf is not a failure");
        assert_eq!(code, 0);
        assert_eq!(lines, vec!["first".to_string()]);
    }

    /// A stub that drains everything still round-trips, so the tolerance does
    /// not hide a genuinely truncated write.
    #[test]
    fn a_draining_fzf_still_receives_every_candidate() {
        let stub = nils_test_support::StubBinDir::new();
        stub.write_exe(
            "fzf",
            r#"#!/bin/bash
count=0
while IFS= read -r _line; do count=$((count+1)); done
printf '%s\n' "$count"
"#,
        );

        let mut input = String::new();
        for index in 0..5_000 {
            input.push_str(&format!("candidate-{index}\n"));
        }

        let (code, lines) =
            with_stub_path(&stub, || run_lines(&input, &[], &[])).expect("a draining fzf succeeds");
        assert_eq!(code, 0);
        assert_eq!(lines, vec!["5000".to_string()]);
    }

    /// A stub that cannot be executed at all is still an error, so tolerating
    /// the closed pipe does not swallow a real spawn or write failure.
    #[test]
    fn a_failing_write_that_is_not_a_closed_pipe_still_fails() {
        for refused in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::WriteZero,
            std::io::ErrorKind::Other,
        ] {
            assert!(
                !is_consumer_closed(&std::io::Error::from(refused)),
                "only a closed pipe is tolerated, not {refused:?}"
            );
        }
        assert!(
            is_consumer_closed(&std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
            "a closed pipe means fzf stopped reading"
        );
    }
}
