# nils-cli

Rust CLI workspace scaffold for building multiple independently packaged binaries.

## Workspace layout
- `crates/nils-common`: shared library crate for cross-CLI helpers
- `crates/cli-template`: minimal binary crate for validating packaging
- `crates/api-testing-core`: shared library crate for the API testing CLIs
- `crates/api-rest`: Rust port of the REST testing CLI
- `crates/api-gql`: Rust port of the GraphQL testing CLI
- `crates/api-test`: Rust port of the API suite runner
- `crates/git-scope`: Rust port of the git-scope CLI
- `crates/git-summary`: Rust port of the git-summary CLI
- `crates/git-lock`: Rust port of the git-lock CLI
- `crates/fzf-cli`: Rust port of personal fzf helper CLI
- `crates/semantic-commit`: Rust port of Codex semantic commit entrypoints

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

## API testing CLIs
See `docs/api-testing/usage.md` for the recommended repo layout and end-to-end examples.

### api-rest
- Example usage:
  - `api-rest call --env staging setup/rest/requests/health.request.json`
  - `api-rest report --case health --request setup/rest/requests/health.request.json --run`
  - `api-rest history --command-only | api-rest report-from-cmd --stdin`
  - `api-rest history`

### api-gql
- Example usage:
  - `api-gql call --env staging setup/graphql/operations/health.graphql`
  - `api-gql report --case health --op setup/graphql/operations/health.graphql --run`
  - `api-gql history --command-only | api-gql report-from-cmd --stdin`
  - `api-gql schema --cat`

### api-test
- Example usage:
  - `api-test run --suite smoke`
  - `api-test run --suite smoke --out out/api-test-runner/results.json --junit out/junit.xml`
  - `api-test summary --in out/api-test-runner/results.json --out out/summary.md`

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
