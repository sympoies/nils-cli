# nils-test-support

## Overview
`nils-test-support` is a test-only helper crate for this workspace (it is `publish = false`).

It provides small utilities to keep tests deterministic when they need to manipulate global state
or stub external commands.

## Utilities
- Global guards
  - `GlobalStateLock`: serialize tests that mutate process-global state (env, cwd, PATH, etc.)
  - `EnvGuard`, `CwdGuard`: RAII guards for temporarily setting env vars / current directory
- Stubbing external tools
  - `StubBinDir`, `write_exe`, `prepend_path`: create a temp bin dir and put it at the front of `PATH`
  - `stubs`: ready-made stub scripts for common external tools (e.g. `fzf`, `bat`, `tree`, `file`, ImageMagick)
- Fixtures
  - `fixtures`: temp repo/layout fixtures for API testing CLIs (REST / GraphQL setup + suite manifests)
- Loopback HTTP server
  - `http`: a tiny in-process loopback server to record requests and return canned responses

## Example
```rust
use nils_test_support::{prepend_path, EnvGuard, GlobalStateLock, StubBinDir};

let lock = GlobalStateLock::new();
let stub_dir = StubBinDir::new();

let _path = prepend_path(&lock, stub_dir.path());
let _env = EnvGuard::set(&lock, "EXAMPLE", "1");
```

