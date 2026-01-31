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

## Shell completions (zsh)

- Completion files:
  - `completions/zsh/_git-scope`
  - `completions/zsh/_api-rest`
  - `completions/zsh/_api-gql`
  - `completions/zsh/_api-test`
- Wrapper scripts: `wrappers/gs`, `wrappers/gsc`, `wrappers/gst`, `wrappers/git-scope`
- Setup:
  - Add `wrappers/` to `PATH`.
  - Add `completions/zsh/` to `fpath` and run `compinit`.
