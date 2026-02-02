# nils-test-support

## Overview
`nils-test-support` is a test-only helper crate for this workspace (it is `publish = false`).

It provides small utilities to keep tests deterministic when they need to manipulate global state
or stub external commands.

## Utilities
- Global guards
  - `GlobalStateLock`: serialize tests that mutate process-global state (env, cwd, PATH, etc.)
  - `EnvGuard`, `CwdGuard`: RAII guards for temporarily setting env vars / current directory
- FS helpers
  - `fs`: write text/bytes/json/executables while ensuring parent dirs exist
- Command runners
  - `cmd`: run binaries with captured output (`CmdOutput`) and flexible options (`CmdOptions`)
- Workspace binaries
  - `bin`: `resolve` finds `CARGO_BIN_EXE_*` or falls back to `target/<profile>/<name>`
- Git helpers
  - `git`: init temp repos (`InitRepoOptions`), run git commands, and commit files
- Stubbing external tools
  - `StubBinDir`, `write_exe`, `prepend_path`: create a temp bin dir and put it at the front of `PATH`
  - `stubs`: ready-made stub scripts for common external tools (e.g. `fzf`, `bat`, `tree`, `file`, ImageMagick/WebP/JPEG)
- Fixtures
  - `fixtures`: REST/GraphQL setup fixtures + suite manifest helpers
- Loopback HTTP server
  - `http`: in-process loopback servers (`LoopbackServer`, `TestServer`) that record requests

## Example
```rust
use nils_test_support::{prepend_path, EnvGuard, GlobalStateLock, StubBinDir};

let lock = GlobalStateLock::new();
let stub_dir = StubBinDir::new();

let _path = prepend_path(&lock, stub_dir.path());
let _env = EnvGuard::set(&lock, "EXAMPLE", "1");
```
