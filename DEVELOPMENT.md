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

## Formatting and linting

- Format check: `cargo fmt --all -- --check`
- Format fix: `cargo fmt --all`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`

## Testing

### Required before committing

- `cargo test -p nils-common`
- `cargo test -p git-scope`
- `zsh -f tests/zsh/completion.test.zsh`

### Targeted tests

- `cargo test -p git-scope --test edge_cases`
- `cargo test -p git-scope --test rendering`
- `cargo test -p git-scope --test commit_mode`
- `cargo test -p git-scope --test print_sources`
- `cargo test -p git-scope --test tracked_prefix`

## Shell completions (zsh)

- Completion file: `completions/zsh/_git-scope`
- Wrapper scripts: `wrappers/gs`, `wrappers/gsc`, `wrappers/gst`, `wrappers/git-scope`
- Setup:
  - Add `wrappers/` to `PATH`.
  - Add `completions/zsh/` to `fpath` and run `compinit`.
