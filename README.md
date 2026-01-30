# nils-cli

Rust CLI workspace scaffold for building multiple independently packaged binaries.

## Workspace layout
- `crates/nils-common`: shared library crate for cross-CLI helpers
- `crates/cli-template`: minimal binary crate for validating packaging
- `crates/git-scope`: Rust port of the git-scope CLI
- `crates/git-summary`: Rust port of the git-summary CLI
- `crates/git-lock`: Rust port of the git-lock CLI
- `crates/fzf-cli`: Rust port of personal fzf helper CLI (from `fzf-tools.zsh`)
- `crates/semantic-commit`: Rust port of Codex semantic commit entrypoints

## Build and run
- `cargo build`
- `cargo build -p cli-template`
- `cargo run -p cli-template -- --help`
- `cargo run -p git-scope -- --help`
- `cargo run -p git-summary -- --help`
- `cargo run -p git-lock -- --help`
- `cargo run -p fzf-cli -- --help`
- `cargo run -p semantic-commit -- --help`
- `cargo test -p nils-common`
- `cargo test -p git-scope`
- `cargo test -p git-summary`
- `cargo test -p git-lock`
- `cargo test -p fzf-cli`
- `cargo test -p semantic-commit`

## Local install (release)
- Build + install all workspace binaries into `~/.local/nils-cli/`:
  - `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh`
- Install only a specific binary:
  - `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh --bin git-scope`
- Add the install dir to `PATH` (example):
  - `export PATH="$HOME/.local/nils-cli:$PATH"`

## git-scope
- Example usage: `git-scope staged`, `git-scope all -p`, `git-scope commit HEAD -p`
- Wrapper aliases (optional): `gs` → `git-scope`, `gsc` → `git-scope commit`, `gst` → `git-scope tracked`

## git-summary
- Example usage: `git-summary all`, `git-summary this-week`, `git-summary 2024-01-01 2024-12-31`

## git-lock
- Example usage: `git-lock lock wip "before refactor"`, `git-lock list`, `git-lock diff alpha beta`

## fzf-cli
- Example usage: `fzf-cli file`, `fzf-cli directory`, `fzf-cli history`, `fzf-cli port`, `fzf-cli process`
- Note: some subcommands print shell commands for `eval` (e.g. `fzf-cli directory` prints a `cd ...`), see `docs/fzf-cli/spec.md`.

## semantic-commit
- Example usage:
  - `semantic-commit staged-context`
  - `semantic-commit commit --message "chore: update thing"`
  - `cat message.txt | semantic-commit commit`

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
