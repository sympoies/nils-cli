# nils-cli

Rust CLI workspace scaffold for building multiple independently packaged binaries.

## Workspace layout
- `crates/nils-common`: shared library crate for cross-CLI helpers
- `crates/cli-template`: minimal binary crate for validating packaging

## Build and run
- `cargo build`
- `cargo build -p cli-template`
- `cargo run -p cli-template -- --help`
- `cargo test -p nils-common`

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
