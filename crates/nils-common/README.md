# nils-common

## Overview
`nils-common` is a small shared library crate for cross-CLI helpers within this workspace.

It is intentionally minimal and only grows when a helper is needed by multiple CLIs.
Behavioral parity is the first constraint: shared helpers must not change user-facing output text,
warning copy, color behavior, or exit-code contracts of consuming CLIs.

## Status
- Implemented and exported today: `fs`, `process` (PATH lookup), `greeting`.
- Planned in Task 1.2 (contract/spec only, not implemented/exported yet): `env`, `shell`, `git`,
  `clipboard`, plus expanded `process` execution helpers.

## Public API (implemented today)
- `greeting(name: &str) -> String`: returns `Hello, {name}!` (used by `cli-template`).
- `fs`:
  - `replace_file(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()>`: rename `from` to `to`, overwriting `to`.
  - `rename_overwrite(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()>`: alias for `replace_file`.
- `process`:
  - `cmd_exists(program: &str) -> bool`: true if `find_in_path` resolves the program.
  - `find_in_path(program: &str) -> Option<std::path::PathBuf>`: resolve from `PATH` (or validate an explicit path).

Notes:
- On Unix, `replace_file` overwrites atomically when `from` and `to` are on the same filesystem.
- On Windows, overwriting falls back to remove + rename when `to` exists (non-atomic).

## Planned module contracts (Task 1.2, specification only)

### `env`
Proposed signatures:
```text
pub fn is_truthy(input: &str) -> bool;
pub fn env_truthy(name: &str) -> bool;
pub fn env_truthy_or(name: &str, default: bool) -> bool;
pub fn no_color_enabled() -> bool;
```

Semantics:
- `is_truthy` is ASCII-case-insensitive and trims surrounding whitespace.
- Accepted truthy tokens are exactly: `1`, `true`, `yes`, `on`.
- `env_truthy*` treats missing variables as `false` unless `default` is provided.
- `no_color_enabled` only checks `NO_COLOR` truthiness; caller crates keep any extra TTY/env rules.

### `shell`
Proposed signatures:
```text
pub enum AnsiStripMode { CsiSgrOnly, CsiAnyTerminator }
pub fn quote_posix_single(input: &str) -> String;
pub fn strip_ansi(input: &str, mode: AnsiStripMode) -> std::borrow::Cow<'_, str>;
```

Semantics:
- `quote_posix_single` returns a POSIX single-quoted shell token for whole-argument safety.
- `strip_ansi` supports explicit parsing modes so CLIs can preserve current parity behavior.

### `process` (expansion)
Existing signatures stay unchanged:
```text
pub fn cmd_exists(program: &str) -> bool;
pub fn find_in_path(program: &str) -> Option<std::path::PathBuf>;
```

Proposed expansion signatures:
```text
pub struct ProcessOutput {
  pub status: std::process::ExitStatus,
  pub stdout: Vec<u8>,
  pub stderr: Vec<u8>,
}
pub enum ProcessError { Io(std::io::Error), NonZero(ProcessOutput) }
pub fn run_output(program: &str, args: &[&str]) -> Result<ProcessOutput, ProcessError>;
pub fn run_checked(program: &str, args: &[&str]) -> Result<(), ProcessError>;
pub fn run_stdout_trimmed(program: &str, args: &[&str]) -> Result<String, ProcessError>;
```

Failure semantics:
- `ProcessError::Io`: spawn/exec/pipe/read failure.
- `ProcessError::NonZero`: command executed but exited non-zero, with raw `status/stdout/stderr`.
- No user-facing message formatting in `nils-common::process`; caller adapters own final text.

### `git`
Proposed signatures:
```text
pub fn is_inside_work_tree(cwd: &std::path::Path) -> Result<bool, ProcessError>;
pub fn repo_root(cwd: &std::path::Path) -> Result<Option<std::path::PathBuf>, ProcessError>;
pub fn rev_parse(cwd: &std::path::Path, args: &[&str]) -> Result<String, ProcessError>;
pub fn rev_parse_opt(cwd: &std::path::Path, args: &[&str]) -> Result<Option<String>, ProcessError>;
```

Semantics:
- `git` module provides only repo probe and command primitives.
- Git UX policy (warnings, parent-selection wording, pager/config policy) stays in each CLI adapter.

### `clipboard`
Proposed signatures:
```text
pub enum ClipboardTool { Pbcopy, WlCopy, Xclip, Xsel, Clip }
pub struct ClipboardPolicy<'a> {
  pub tool_order: &'a [ClipboardTool],
  pub warn_on_failure: bool,
}
pub enum ClipboardOutcome { Copied(ClipboardTool), SkippedNoTool, SkippedFailure }
pub fn copy_best_effort(text: &str, policy: &ClipboardPolicy<'_>) -> ClipboardOutcome;
```

Semantics:
- `copy_best_effort` is best-effort and never panics.
- Tool order and warn/silent behavior are policy inputs so crates can keep current UX parity.

## Compatibility and adaptation rules
- Keep crate-specific warning/error copy in local adapters (including emoji/prefix formatting).
- Keep crate-specific exit-code mapping in local adapters.
- Keep crate-specific color policy in local adapters (`NO_COLOR` is shared baseline only).
- Keep crate-specific shell quote style selection in local adapters when user-visible snippets must
  stay byte-for-byte identical.
- Keep crate-specific git command composition in local adapters (`GIT_PAGER`, config, trim policy).
- `nils-common` stays domain-neutral and must not absorb CLI command/business logic.

## Examples
Greeting (used by `cli-template`):
```rust
let greeting = nils_common::greeting("Nils");
assert_eq!(greeting, "Hello, Nils!");
```

Find an executable:
```rust
use nils_common::process;

assert!(process::cmd_exists("git"));
let git = process::find_in_path("git").expect("git on PATH");
assert!(git.ends_with("git"));
```

Replace a file (overwrite destination):
```rust
use std::fs;

let dir = std::env::temp_dir();
let pid = std::process::id();

let from = dir.join(format!("nils-common-{pid}.tmp"));
let to = dir.join(format!("nils-common-{pid}.txt"));

fs::write(&from, "new").unwrap();
fs::write(&to, "old").unwrap();

nils_common::fs::replace_file(&from, &to).unwrap();
assert_eq!(fs::read_to_string(&to).unwrap(), "new");

let _ = fs::remove_file(&to);
```
