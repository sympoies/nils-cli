---
name: nils-cli-verify-required-checks
description: Run the required lint + test commands from DEVELOPMENT.md (fmt, clippy, cargo test, zsh completion).
---

# Nils CLI Verify Required Checks

## Contract

Prereqs:

- Run inside the `nils-cli` git work tree (the script resolves the repo root via `git`).
- `cargo` and a Rust toolchain available on `PATH` (including `rustfmt` and `clippy` components).
- `zsh` available on `PATH`.

Inputs:

- Optional flag: `--docs-only` (documentation-only fast path).

Outputs:

- Runs the required pre-delivery checks from `DEVELOPMENT.md`:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `bash scripts/ci/test-stale-audit.sh --strict`
  - `bash scripts/ci/completion-asset-audit.sh --strict`
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `bash scripts/ci/completion-flag-parity-audit.sh --strict`
  - `zsh -f tests/zsh/completion.test.zsh`
- In `--docs-only` mode, runs only:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
- Prints the failing command (if any) and exits non-zero on failure.

Exit codes:

- `0`: all checks passed
- `1`: a check failed
- `2`: usage error (invalid arguments) or missing prerequisites

Failure modes:

- Not in a git work tree (cannot resolve repo root).
- Missing required tools on `PATH` (`git`, `cargo`, `zsh`, `rg`).
- Any of the required lint/tests fail.

## Scripts (only entrypoints)

- `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Workflow

- Run before you claim a task is done.
- For docs-only changes (`README.md` / `docs/**` / `*.md` only), prefer:
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- If it fails, fix the reported issue and re-run until it exits `0`.
