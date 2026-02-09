use crate::process;
use std::io;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardTool {
    Pbcopy,
    WlCopy,
    Xclip,
    Xsel,
    Clip,
}

impl ClipboardTool {
    fn program(self) -> &'static str {
        match self {
            Self::Pbcopy => "pbcopy",
            Self::WlCopy => "wl-copy",
            Self::Xclip => "xclip",
            Self::Xsel => "xsel",
            Self::Clip => "clip",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Pbcopy => &[],
            Self::WlCopy => &[],
            Self::Xclip => &["-selection", "clipboard"],
            Self::Xsel => &["--clipboard", "--input"],
            Self::Clip => &[],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClipboardPolicy<'a> {
    pub tool_order: &'a [ClipboardTool],
}

impl<'a> ClipboardPolicy<'a> {
    pub const fn new(tool_order: &'a [ClipboardTool]) -> Self {
        Self { tool_order }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardOutcome {
    Copied(ClipboardTool),
    SkippedNoTool,
    SkippedFailure,
}

pub fn copy_best_effort(text: &str, policy: &ClipboardPolicy<'_>) -> ClipboardOutcome {
    let mut saw_tool = false;

    for tool in policy.tool_order {
        let program = tool.program();
        if !process::cmd_exists(program) {
            continue;
        }

        saw_tool = true;
        if pipe_to_command(program, tool.args(), text).is_ok() {
            return ClipboardOutcome::Copied(*tool);
        }
    }

    if saw_tool {
        ClipboardOutcome::SkippedFailure
    } else {
        ClipboardOutcome::SkippedNoTool
    }
}

fn pipe_to_command(cmd: &str, args: &[&str], text: &str) -> io::Result<()> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{cmd} exited with status {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    fn write_clipboard_stub(stubs: &StubBinDir, name: &str) {
        stubs.write_exe(
            name,
            &format!(
                r#"#!/bin/bash
set -euo pipefail
chosen="${{CLIPBOARD_TOOL_CHOSEN:?CLIPBOARD_TOOL_CHOSEN is required}}"
payload="${{CLIPBOARD_PAYLOAD_OUT:?CLIPBOARD_PAYLOAD_OUT is required}}"
printf "%s" "{name}" > "$chosen"
/bin/cat > "$payload"
"#
            ),
        );
    }

    #[test]
    fn copy_best_effort_respects_tool_order() {
        let lock = GlobalStateLock::new();
        let stubs = StubBinDir::new();
        write_clipboard_stub(&stubs, "pbcopy");
        write_clipboard_stub(&stubs, "wl-copy");
        write_clipboard_stub(&stubs, "xclip");
        write_clipboard_stub(&stubs, "xsel");

        let out_dir = TempDir::new().expect("tempdir");
        let chosen_path = out_dir.path().join("chosen.txt");
        let payload_path = out_dir.path().join("payload.txt");

        let _path_guard = prepend_path(&lock, stubs.path());
        let chosen_path_str = chosen_path.to_string_lossy();
        let payload_path_str = payload_path.to_string_lossy();
        let _chosen_guard = EnvGuard::set(&lock, "CLIPBOARD_TOOL_CHOSEN", chosen_path_str.as_ref());
        let _payload_guard =
            EnvGuard::set(&lock, "CLIPBOARD_PAYLOAD_OUT", payload_path_str.as_ref());

        let tool_order = [
            ClipboardTool::Pbcopy,
            ClipboardTool::WlCopy,
            ClipboardTool::Xclip,
            ClipboardTool::Xsel,
        ];
        let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));
        assert_eq!(outcome, ClipboardOutcome::Copied(ClipboardTool::Pbcopy));
        assert_eq!(
            fs::read_to_string(chosen_path).expect("chosen"),
            "pbcopy".to_string()
        );
        assert_eq!(fs::read_to_string(payload_path).expect("payload"), "hello");
    }

    #[test]
    fn copy_best_effort_honors_custom_order() {
        let lock = GlobalStateLock::new();
        let stubs = StubBinDir::new();
        write_clipboard_stub(&stubs, "xclip");
        write_clipboard_stub(&stubs, "xsel");

        let out_dir = TempDir::new().expect("tempdir");
        let chosen_path = out_dir.path().join("chosen.txt");
        let payload_path = out_dir.path().join("payload.txt");

        let _path_guard = prepend_path(&lock, stubs.path());
        let chosen_path_str = chosen_path.to_string_lossy();
        let payload_path_str = payload_path.to_string_lossy();
        let _chosen_guard = EnvGuard::set(&lock, "CLIPBOARD_TOOL_CHOSEN", chosen_path_str.as_ref());
        let _payload_guard =
            EnvGuard::set(&lock, "CLIPBOARD_PAYLOAD_OUT", payload_path_str.as_ref());

        let tool_order = [ClipboardTool::Xsel, ClipboardTool::Xclip];
        let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));
        assert_eq!(outcome, ClipboardOutcome::Copied(ClipboardTool::Xsel));
        assert_eq!(fs::read_to_string(chosen_path).expect("chosen"), "xsel");
        assert_eq!(fs::read_to_string(payload_path).expect("payload"), "hello");
    }

    #[test]
    fn copy_best_effort_returns_skipped_when_no_tool_exists() {
        let tool_order: [ClipboardTool; 0] = [];
        let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));
        assert_eq!(outcome, ClipboardOutcome::SkippedNoTool);
    }

    #[test]
    fn copy_best_effort_falls_back_when_first_tool_fails() {
        let lock = GlobalStateLock::new();
        let stubs = StubBinDir::new();
        stubs.write_exe(
            "pbcopy",
            r#"#!/bin/bash
exit 1
"#,
        );
        stubs.write_exe(
            "xclip",
            r#"#!/bin/bash
set -euo pipefail
payload="${CLIPBOARD_PAYLOAD_OUT:?CLIPBOARD_PAYLOAD_OUT is required}"
/bin/cat > "$payload"
"#,
        );

        let out_dir = TempDir::new().expect("tempdir");
        let payload_path = out_dir.path().join("payload.txt");

        let _path_guard = prepend_path(&lock, stubs.path());
        let payload_path_str = payload_path.to_string_lossy();
        let _payload_guard =
            EnvGuard::set(&lock, "CLIPBOARD_PAYLOAD_OUT", payload_path_str.as_ref());

        let tool_order = [ClipboardTool::Pbcopy, ClipboardTool::Xclip];
        let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));
        assert_eq!(outcome, ClipboardOutcome::Copied(ClipboardTool::Xclip));
        assert_eq!(fs::read_to_string(payload_path).expect("payload"), "hello");
    }

    #[test]
    fn copy_best_effort_returns_skipped_failure_when_all_tools_fail() {
        let lock = GlobalStateLock::new();
        let stubs = StubBinDir::new();
        stubs.write_exe(
            "pbcopy",
            r#"#!/bin/bash
exit 1
"#,
        );
        stubs.write_exe(
            "xclip",
            r#"#!/bin/bash
exit 2
"#,
        );

        let _path_guard = prepend_path(&lock, stubs.path());

        let tool_order = [ClipboardTool::Pbcopy, ClipboardTool::Xclip];
        let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));
        assert_eq!(outcome, ClipboardOutcome::SkippedFailure);
    }

    #[test]
    fn copy_best_effort_passes_tool_specific_args() {
        let lock = GlobalStateLock::new();
        let stubs = StubBinDir::new();
        stubs.write_exe(
            "xclip",
            r#"#!/bin/bash
set -euo pipefail
args_out="${CLIPBOARD_ARGS_OUT:?CLIPBOARD_ARGS_OUT is required}"
printf "%s" "$*" > "$args_out"
cat > /dev/null
"#,
        );

        let out_dir = TempDir::new().expect("tempdir");
        let args_path = out_dir.path().join("args.txt");

        let _path_guard = prepend_path(&lock, stubs.path());
        let args_path_str = args_path.to_string_lossy();
        let _args_guard = EnvGuard::set(&lock, "CLIPBOARD_ARGS_OUT", args_path_str.as_ref());

        let tool_order = [ClipboardTool::Xclip];
        let outcome = copy_best_effort("hello", &ClipboardPolicy::new(&tool_order));
        assert_eq!(outcome, ClipboardOutcome::Copied(ClipboardTool::Xclip));
        assert_eq!(
            fs::read_to_string(args_path).expect("args"),
            "-selection clipboard"
        );
    }
}
