use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::anyhow;

use crate::Result;

pub fn read_response_bytes(response: &str, stdin: &mut dyn Read) -> Result<Vec<u8>> {
    if response == "-" {
        let mut buf = Vec::new();
        stdin
            .read_to_end(&mut buf)
            .map_err(|_| anyhow!("error: failed to read response from stdin"))?;
        return Ok(buf);
    }

    let resp_path = PathBuf::from(response);
    if !resp_path.is_file() {
        return Err(anyhow!("Response file not found: {}", resp_path.display()));
    }

    std::fs::read(&resp_path).map_err(|_| {
        anyhow!(
            "error: failed to read response file: {}",
            resp_path.display()
        )
    })
}

pub fn maybe_print_failure_body_to_stderr(
    body: &[u8],
    max_bytes: usize,
    stdout_is_tty: bool,
    stderr: &mut dyn Write,
) {
    if stdout_is_tty || body.is_empty() {
        return;
    }

    if serde_json::from_slice::<serde_json::Value>(body).is_ok() {
        return;
    }

    let _ = writeln!(stderr, "Response body (non-JSON; first {max_bytes} bytes):");
    let _ = stderr.write_all(&body[..body.len().min(max_bytes)]);
    let _ = writeln!(stderr);
}

#[cfg(test)]
mod tests {
    use super::maybe_print_failure_body_to_stderr;

    #[test]
    fn maybe_print_failure_body_skips_when_stdout_is_tty() {
        let mut stderr = Vec::new();
        maybe_print_failure_body_to_stderr(b"not-json", 16, true, &mut stderr);
        assert!(stderr.is_empty());
    }

    #[test]
    fn maybe_print_failure_body_skips_when_response_is_json() {
        let mut stderr = Vec::new();
        maybe_print_failure_body_to_stderr(br#"{"ok":true}"#, 16, false, &mut stderr);
        assert!(stderr.is_empty());
    }

    #[test]
    fn maybe_print_failure_body_prints_non_json_preview() {
        let mut stderr = Vec::new();
        maybe_print_failure_body_to_stderr(b"abcdef", 4, false, &mut stderr);
        let text = String::from_utf8(stderr).expect("utf8");
        assert!(text.contains("Response body (non-JSON; first 4 bytes):"));
        assert!(text.contains("abcd"));
    }
}
