# Development Guide

## Setup

- If Rust/cargo (or required cargo tools) are not installed yet, run:
  - `scripts/setup-rust-tooling.sh`
- Manual setup fallback:
  - Install Rust via rustup (stable toolchain).
  - Ensure `rustfmt` and `clippy` components are installed:
    - `rustup component add rustfmt clippy`
- Optional tools for full CLI output fidelity:
  - `tree` (directory tree rendering)
  - `file` (binary/text detection)

## Build and run

- Build workspace: `cargo build`
- Run CLI template: `cargo run -p nils-cli-template -- --help`
- Run git-scope: `cargo run -p nils-git-scope -- --help`

## CLI version policy

- Every user-facing CLI must expose a root `-V, --version` flag.
- For clap-based CLIs, set `#[command(version)]` on the root `Parser`.
- `--help` output should include `-V, --version` in options/help text (auto-generated or custom).

## Local install (release)

- Build + install all workspace binaries into `~/.local/nils-cli/`:
  - `./.agents/skills/nils-cli-install-local-release-binaries/scripts/nils-cli-install-local-release-binaries.sh`

## Formatting and linting

- Format check: `cargo fmt --all -- --check`
- Format fix: `cargo fmt --all`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`

## Testing

### Test conventions

- In Rust tests, prefer `pretty_assertions::{assert_eq, assert_ne}` (more readable diffs on failure).

## Documentation placement

- Documentation placement policy: `docs/specs/crate-docs-placement-policy.md`.
- Contributors MUST classify docs as `workspace-level` or `crate-local` before adding/moving files.
- `crate-local` docs MUST be placed under `crates/<crate>/docs/...` canonical paths.
- Root `docs/` MUST be used only for `workspace-level` docs.
- Legacy crate-owned root paths are redirect stubs (no deprecation sunset) and MUST NOT hold canonical content.

## Completion governance

- Canonical completion governance runbook: `docs/runbooks/cli-completion-development-standard.md`.
- When completion/alias code changes, follow that runbook for single-path clap-first policy and completion-focused validation.
- Completion mode policy is `clap-first` with a single generated completion path.
- Release packaging must ship both `completions/zsh/` and `completions/bash/`, including alias files `completions/zsh/aliases.zsh` and `completions/bash/aliases.bash`.

### Required before committing

- All commands in **Formatting and linting** must pass.
- `cargo test --workspace`
- Completion verification commands from `docs/runbooks/cli-completion-development-standard.md`:
  - `zsh -f tests/zsh/completion.test.zsh`
  - `zsh -n completions/zsh/_<cli>`
  - `bash -n completions/bash/<cli>`
- Documentation placement for changed Markdown files MUST comply with
  `docs/specs/crate-docs-placement-policy.md`.
- `bash scripts/ci/docs-placement-audit.sh --strict`
- Coverage must be **>= 85.00%** total line coverage:
  - `mkdir -p target/coverage`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Or run the single entrypoint for fmt/clippy/tests: `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` (it pre-creates `target/coverage`; still run coverage commands above)
- Docs-only fast path: if every changed file is documentation-only (`*.md`, `docs/**`, `crates/*/docs/**`, plus root docs like `README.md`, `DEVELOPMENT.md`), run:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
  - In this mode, full workspace lint/test/coverage checks may be skipped.

### CI-style test reporting (optional)

- If `cargo nextest` is missing, run `scripts/setup-rust-tooling.sh`
- Run CI-style tests + generate JUnit: `cargo nextest run --profile ci --workspace` (writes `target/nextest/ci/junit.xml`)
- Note: nextest does not run doctests; run separately: `cargo test --workspace --doc`

## Coverage

- Policy: total line coverage must be **>= 85.00%** (enforced in CI).

- Prereqs:

  ```bash
  scripts/setup-rust-tooling.sh
  ```

- Generate coverage artifacts (recommended; matches CI runner):

  ```bash
  mkdir -p target/coverage
  cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info
  cargo llvm-cov report --html --output-dir target/coverage
  ```

- Outputs:
  - `target/coverage/lcov.info`
  - `target/coverage/html/index.html`
- Note: doctests are **not included** in coverage initially; still run doctests for correctness: `cargo test --workspace --doc`

## Shell completions

For completion implementation workflow and contributor validation requirements, use
`docs/runbooks/cli-completion-development-standard.md`. The setup notes below focus on local shell integration.

### Zsh

- Completion files: `completions/zsh/`
- Optional aliases (Zsh): `completions/zsh/aliases.zsh`
- Wrapper scripts (dev-only): `wrappers/`
- Setup:
  - Add `wrappers/` to `PATH`.
  - Add `completions/zsh/` to `fpath` and run `compinit`.
  - Optional: `source completions/zsh/aliases.zsh`

### Bash

- Completion files: `completions/bash/`
- Optional aliases (Bash): `completions/bash/aliases.bash`
- Setup:
  - Install `bash-completion` (recommended), then copy `completions/bash/<command>` into your completions directory (example: `~/.local/share/bash-completion/completions/`).
  - Or: source the desired files from your `~/.bashrc`.
  - Optional: `source completions/bash/aliases.bash`
