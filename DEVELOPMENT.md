# Development Guide

## Setup

- Install Rust via rustup (stable toolchain).
- Ensure `rustfmt` and `clippy` components are installed:
  - `rustup component add rustfmt clippy`
- Optional tools for full CLI output fidelity:
  - `tree` (directory tree rendering)
  - `file` (binary/text detection)

## Build and run

- Build workspace: `cargo build`
- Run CLI template: `cargo run -p cli-template -- --help`
- Run git-scope: `cargo run -p git-scope -- --help`

## Local install (release)

- Build + install all workspace binaries into `~/.local/nils-cli/`:
  - `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh`

## Formatting and linting

- Format check: `cargo fmt --all -- --check`
- Format fix: `cargo fmt --all`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`

## Testing

### Test conventions

- In Rust tests, prefer `pretty_assertions::{assert_eq, assert_ne}` (more readable diffs on failure).

### Required before committing

- All commands in **Formatting and linting** must pass.
- `cargo test --workspace`
- `zsh -f tests/zsh/completion.test.zsh`
- Coverage must be **>= 80.00%** total line coverage:
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- Or run the single entrypoint for fmt/clippy/tests: `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh` (still run coverage commands above)

### CI-style test reporting (optional)

- Install `cargo-nextest`: `cargo install cargo-nextest --locked`
- Run CI-style tests + generate JUnit: `cargo nextest run --profile ci --workspace` (writes `target/nextest/ci/junit.xml`)
- Note: nextest does not run doctests; run separately: `cargo test --workspace --doc`

## Coverage

- Policy: total line coverage must be **>= 80.00%** (enforced in CI).

- Prereqs:

  ```bash
  rustup component add llvm-tools-preview
  cargo install cargo-llvm-cov --locked
  cargo install cargo-nextest --locked
  ```

- Generate coverage artifacts (recommended; matches CI runner):

  ```bash
  cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info
  cargo llvm-cov report --html --output-dir target/coverage
  ```

- Outputs:
  - `target/coverage/lcov.info`
  - `target/coverage/html/index.html`
- Note: doctests are **not included** in coverage initially; still run doctests for correctness: `cargo test --workspace --doc`

## Shell completions

### Zsh

- Completion files:
  - `completions/zsh/_git-scope`
  - `completions/zsh/_git-summary`
  - `completions/zsh/_git-lock`
  - `completions/zsh/_fzf-cli`
  - `completions/zsh/_codex-cli`
  - `completions/zsh/_semantic-commit`
  - `completions/zsh/_plan-tooling`
  - `completions/zsh/_api-rest`
  - `completions/zsh/_api-gql`
  - `completions/zsh/_api-test`
- Optional aliases (Zsh): `completions/zsh/aliases.zsh`
- Wrapper scripts (dev-only): `wrappers/plan-tooling`, `wrappers/api-rest`, `wrappers/api-gql`, `wrappers/api-test`, `wrappers/git-scope`, `wrappers/codex-cli`, `wrappers/fzf-cli`
- Setup:
  - Add `wrappers/` to `PATH`.
  - Add `completions/zsh/` to `fpath` and run `compinit`.
  - Optional: `source completions/zsh/aliases.zsh`

### Bash

- Completion files:
  - `completions/bash/git-scope`
  - `completions/bash/git-summary`
  - `completions/bash/git-lock`
  - `completions/bash/fzf-cli`
  - `completions/bash/codex-cli`
  - `completions/bash/semantic-commit`
  - `completions/bash/plan-tooling`
  - `completions/bash/api-rest`
  - `completions/bash/api-gql`
  - `completions/bash/api-test`
- Optional aliases (Bash): `completions/bash/aliases.bash`
- Setup:
  - Install `bash-completion` (recommended), then copy `completions/bash/<command>` into your completions directory (example: `~/.local/share/bash-completion/completions/`).
  - Or: source the desired files from your `~/.bashrc`.
  - Optional: `source completions/bash/aliases.bash`
