# nils-cli

Rust CLI workspace scaffold for building multiple independently packaged binaries.

## Workspace layout
- `crates/nils-common`: shared library crate for cross-CLI helpers
- `crates/cli-template`: minimal binary crate for validating packaging
- `crates/git-scope`: Rust port of the git-scope CLI
- `crates/git-summary`: Rust port of the git-summary CLI
- `crates/git-lock`: Rust port of the git-lock CLI

## Build and run
- `cargo build`
- `cargo build -p cli-template`
- `cargo run -p cli-template -- --help`
- `cargo run -p git-scope -- --help`
- `cargo run -p git-summary -- --help`
- `cargo run -p git-lock -- --help`
- `cargo test -p nils-common`
- `cargo test -p git-scope`
- `cargo test -p git-summary`
- `cargo test -p git-lock`

## git-scope
- Example usage: `git-scope staged`, `git-scope all -p`, `git-scope commit HEAD -p`
- Wrapper aliases (optional): `gs` → `git-scope`, `gsc` → `git-scope commit`, `gst` → `git-scope tracked`

## git-summary
- Example usage: `git-summary all`, `git-summary this-week`, `git-summary 2024-01-01 2024-12-31`

## git-lock
- Example usage: `git-lock lock wip "before refactor"`, `git-lock list`, `git-lock diff alpha beta`

## Adding a new CLI crate
1. Create a new binary crate under `crates/`:
   - `cargo new crates/<cli-name> --bin`
2. Add the crate path to the workspace `members` list in `Cargo.toml`.
3. Use shared dependencies via `workspace = true` in the new crate's `Cargo.toml`.
4. Build or run the new CLI with `cargo build -p <cli-name>` or `cargo run -p <cli-name> -- ...`.

## Zsh wrappers and completions
This repo keeps optional zsh wrapper scripts and completion assets in-repo. See
`docs/completions-strategy.md` for the layout and integration steps. For zsh completion
setup and wrapper installation:
- Add `wrappers/` to your PATH (or symlink the wrappers into a bin directory).
- Add `completions/zsh/` to your `fpath` and run `compinit`.

## License

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

This project is licensed under the MIT License. See [LICENSE](LICENSE).
