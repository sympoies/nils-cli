# nils-common

## Overview
`nils-common` is a small shared library crate for cross-CLI helpers within this workspace.

It is intentionally minimal and only grows when a helper is needed by multiple CLIs.

## Public API
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
