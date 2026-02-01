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

### Required before committing

- All commands in **Formatting and linting** must pass.
- `cargo test --workspace`
- `zsh -f tests/zsh/completion.test.zsh`
- Or run the single entrypoint: `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

### CI-style test reporting (optional)

- Install `cargo-nextest`: `cargo install cargo-nextest --locked`
- Run CI-style tests + generate JUnit: `cargo nextest run --profile ci --workspace` (writes `target/nextest/ci/junit.xml`)
- Note: nextest does not run doctests; run separately: `cargo test --workspace --doc`

## Coverage (optional)

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

## Shell completions (zsh)

- Completion files:
  - `completions/zsh/_git-scope`
  - `completions/zsh/_plan-tooling`
  - `completions/zsh/_api-rest`
  - `completions/zsh/_api-gql`
  - `completions/zsh/_api-test`
- Wrapper scripts: `wrappers/plan-tooling`, `wrappers/api-rest`, `wrappers/api-gql`, `wrappers/api-test`, `wrappers/gs`, `wrappers/gsc`, `wrappers/gst`, `wrappers/git-scope`
- Setup:
  - Add `wrappers/` to `PATH`.
  - Add `completions/zsh/` to `fpath` and run `compinit`.
