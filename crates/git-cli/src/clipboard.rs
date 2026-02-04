use crate::util;
use anyhow::{Context, Result};
use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn set_clipboard_best_effort(text: &str) -> Result<()> {
    if env::var("GIT_CLI_FIXTURE_CLIPBOARD_MODE").ok().as_deref() == Some("missing") {
        eprintln!("⚠️  No clipboard tool found (requires pbcopy, xclip, or xsel)");
        return Ok(());
    }

    if util::cmd_exists("pbcopy") {
        let _ = pipe_to_command("pbcopy", &[], text);
        return Ok(());
    }
    if util::cmd_exists("wl-copy") {
        let _ = pipe_to_command("wl-copy", &[], text);
        return Ok(());
    }
    if util::cmd_exists("xclip") {
        let _ = pipe_to_command("xclip", &["-selection", "clipboard"], text);
        return Ok(());
    }
    if util::cmd_exists("xsel") {
        let _ = pipe_to_command("xsel", &["--clipboard", "--input"], text);
        return Ok(());
    }

    eprintln!("⚠️  No clipboard tool found (requires pbcopy, xclip, or xsel)");
    Ok(())
}

fn pipe_to_command(cmd: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {cmd}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes()).context("write stdin")?;
    }
    let _ = child.wait();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{prepend_path, EnvGuard, GlobalStateLock, StubBinDir};
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn set_clipboard_best_effort_prefers_pbcopy_when_present() {
        let lock = GlobalStateLock::new();

        let stubs = StubBinDir::new();
        let out_dir = TempDir::new().expect("tempdir");
        let out_path = out_dir.path().join("pbcopy.out");

        stubs.write_exe(
            "pbcopy",
            r#"#!/bin/bash
set -euo pipefail
out="${PB_COPY_OUT:?PB_COPY_OUT is required}"
/bin/cat > "$out"
"#,
        );

        let _path_guard: EnvGuard = prepend_path(&lock, stubs.path());
        let out_path_str = out_path.to_string_lossy();
        let _out_guard = EnvGuard::set(&lock, "PB_COPY_OUT", out_path_str.as_ref());

        set_clipboard_best_effort("hello").expect("copy");
        let out = fs::read_to_string(out_path).expect("read stub output");
        assert_eq!(out, "hello");
    }

    #[test]
    fn set_clipboard_best_effort_uses_xsel_when_present() {
        let lock = GlobalStateLock::new();

        let stubs = StubBinDir::new();
        let out_dir = TempDir::new().expect("tempdir");
        let out_path = out_dir.path().join("xsel.out");

        stubs.write_exe(
            "xsel",
            r#"#!/bin/bash
set -euo pipefail
out="${XSEL_OUT:?XSEL_OUT is required}"
/bin/cat > "$out"
"#,
        );

        let _path_guard = EnvGuard::set(&lock, "PATH", &stubs.path_str());
        let out_path_str = out_path.to_string_lossy();
        let _out_guard = EnvGuard::set(&lock, "XSEL_OUT", out_path_str.as_ref());

        set_clipboard_best_effort("hello").expect("copy");
        let out = fs::read_to_string(out_path).expect("read stub output");
        assert_eq!(out, "hello");
    }
}
