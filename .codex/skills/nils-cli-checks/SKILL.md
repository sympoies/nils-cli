---
name: nils-cli-checks
description: Run the required lint + test commands from DEVELOPMENT.md (fmt, clippy, cargo test, zsh completion).
---

# Nils CLI Checks

## Contract

Prereqs:

- Run inside the `nils-cli` git work tree (the script resolves the repo root via `git`).
- `cargo` and a Rust toolchain available on `PATH` (including `rustfmt` and `clippy` components).
- `zsh` available on `PATH`.

Inputs:

- None.

Outputs:

- Runs the required pre-delivery checks from `DEVELOPMENT.md`:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `zsh -f tests/zsh/completion.test.zsh`
- Prints the failing command (if any) and exits non-zero on failure.

Exit codes:

- `0`: all checks passed
- `1`: a check failed
- `2`: usage error (invalid arguments) or missing prerequisites

Failure modes:

- Not in a git work tree (cannot resolve repo root).
- Missing required tools on `PATH` (`git`, `cargo`, `zsh`).
- Any of the required lint/tests fail.

## Scripts (only entrypoints)

- `.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Workflow

- Run before you claim a task is done.
- If it fails, fix the reported issue and re-run until it exits `0`.
