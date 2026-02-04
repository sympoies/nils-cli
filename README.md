# nils-cli

[![Coverage](https://raw.githubusercontent.com/graysurf/nils-cli/coverage-badge/badges/coverage.svg)](https://github.com/graysurf/nils-cli/actions/workflows/ci.yml)

Rust CLI workspace scaffold for building multiple independently packaged binaries.

## Workspace layout
Each crate is either a standalone CLI binary or a shared library used across the workspace.

- [crates/nils-common](crates/nils-common): Shared cross-CLI utilities (small helpers that don’t belong to a specific binary).
- [crates/nils-term](crates/nils-term): Terminal UX helpers (TTY detection + progress rendering on stderr).
- [crates/nils-test-support](crates/nils-test-support): Test-only helpers for deterministic workspace integration tests.
- [crates/cli-template](crates/cli-template): Minimal example CLI for validating packaging and new-crate patterns.
- [crates/api-testing-core](crates/api-testing-core): Shared library for the API testing CLIs (config/auth, history, reporting).
- [crates/api-rest](crates/api-rest): REST request runner from JSON request specs, with history + Markdown reports.
- [crates/api-gql](crates/api-gql): GraphQL operation runner for `.graphql` files (variables, history, reports, schema).
- [crates/api-test](crates/api-test): Suite runner that orchestrates REST/GraphQL cases and outputs JSON (and optional JUnit).
- [crates/git-scope](crates/git-scope): Git change inspector (tracked/staged/unstaged/untracked/commit) with tree + optional file printing.
- [crates/git-summary](crates/git-summary): Per-author contribution summaries over a date range (adds/dels/net/commits).
- [crates/git-lock](crates/git-lock): Label-based commit locks per repo (lock/list/diff/unlock/tag).
- [crates/fzf-cli](crates/fzf-cli): Interactive `fzf` toolbox for files, Git, processes, ports, and shell history.
- [crates/codex-cli](crates/codex-cli): Helper CLI for Codex workflows (auth, diagnostics, agent wrappers, starship snippets).
- [crates/semantic-commit](crates/semantic-commit): Helper CLI for generating staged context and creating semantic commits.
- [crates/plan-tooling](crates/plan-tooling): Plan Format v1 tooling CLI (to-json/validate/batches/scaffold).
- [crates/image-processing](crates/image-processing): Batch image transformation CLI (resize/crop/optimize) with JSON/report outputs.

## Local install (release)
- Build + install all workspace binaries into `~/.local/nils-cli/`:
  - `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh`
- Install only a specific binary:
  - `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh --bin git-scope`
- Add the install dir to `PATH` (example):
  - `export PATH="$HOME/.local/nils-cli:$PATH"`

## GitHub Releases (prebuilt binaries)

This repo can publish prebuilt tarballs via GitHub Releases for both:
- x86_64 (amd64)
- aarch64 (arm64)

To trigger a release build, push a tag like `v0.1.7`:
- `git tag -a v0.1.7 -m "v0.1.7"`
- `git push origin v0.1.7`

Then download the matching `nils-cli-<tag>-<target>.tar.gz` asset, extract it, and add
`<extract_dir>/bin` to your `PATH`.

For zsh completions, add `<extract_dir>/completions/zsh` to your `fpath` and run `compinit`.
Optional: source `<extract_dir>/completions/zsh/aliases.zsh` to enable `gs*`/`cx*`/`ff*` aliases.

For bash completions, copy `<extract_dir>/completions/bash/<command>` into your bash-completion directory
(example: `~/.local/share/bash-completion/completions/`) or source it from your shell init.
Optional: source `<extract_dir>/completions/bash/aliases.bash` to enable `gs*`/`cx*`/`ff*` aliases.

## git-scope
- Example usage: `git-scope staged`, `git-scope all -p`, `git-scope commit HEAD -p`
- Optional aliases (opt-in; Zsh: `completions/zsh/aliases.zsh`, Bash: `completions/bash/aliases.bash`):
  - `gs` → `git-scope`
  - `gst` → `git-scope tracked`
  - `gss` → `git-scope staged`
  - `gsu` → `git-scope unstaged`
  - `gsa` → `git-scope all`
  - `gsun` → `git-scope untracked`
  - `gsc` → `git-scope commit`
  - `gsh` → `git-scope help`

## git-summary
- Example usage: `git-summary all`, `git-summary this-week`, `git-summary 2024-01-01 2024-12-31`

## git-lock
- Example usage: `git-lock lock wip "before refactor"`, `git-lock list`, `git-lock diff alpha beta`

## fzf-cli
- Example usage: `fzf-cli file`, `fzf-cli directory`, `fzf-cli history`, `fzf-cli port`, `fzf-cli process`
- Note: some subcommands print shell commands for `eval` (e.g. `fzf-cli directory` prints a `cd ...`), see `crates/fzf-cli/README.md`.
- Optional aliases (opt-in): `ff*` (Zsh: `completions/zsh/aliases.zsh`, Bash: `completions/bash/aliases.bash`); `ffd` and `ffh` are functions that `eval` the emitted command.

## codex-cli
- Docs: `crates/codex-cli/README.md`
- Example usage: `codex-cli auth current`, `codex-cli diag rate-limits --one-line`
- Optional aliases (opt-in): `cx*` (Zsh: `completions/zsh/aliases.zsh`, Bash: `completions/bash/aliases.bash`).

## semantic-commit
- Example usage:
  - `semantic-commit staged-context`
  - `semantic-commit commit --message "chore: update thing"`
  - `cat message.txt | semantic-commit commit`

## plan-tooling
- Example usage:
  - `plan-tooling to-json --file docs/plans/plan-tooling-cli-consolidation-plan.md --pretty | jq .`
  - `plan-tooling validate`
  - `plan-tooling batches --file docs/plans/plan-tooling-cli-consolidation-plan.md --sprint 1 --format text`
  - `plan-tooling scaffold --slug my-new-cli --title "My new CLI plan"`

## API testing CLIs
See `crates/api-testing-core/README.md` for the recommended repo layout and end-to-end examples.

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
  - `api-test run --suite smoke --out out/api-test-runner/results.json --junit out/api-test-runner/junit.xml`
  - `api-test summary --in out/api-test-runner/results.json --out out/api-test-runner/summary.md`

## Adding a new CLI crate
1. Create a new binary crate under `crates/`:
   - `cargo new crates/<cli-name> --bin`
2. Add the crate path to the workspace `members` list in `Cargo.toml`.
3. Use shared dependencies via `workspace = true` in the new crate's `Cargo.toml`.
4. Build or run the new CLI with `cargo build -p <cli-name>` or `cargo run -p <cli-name> -- ...`.
5. Verify packaging picks it up (both local install + GitHub Releases use the same discovery):
   - `python3 scripts/workspace-bins.py | rg "^<cli-name>$"`

## Shell wrappers and completions
This repo keeps optional wrapper scripts and completion assets in-repo.

Decision:
- Keep completion and wrapper assets under `completions/` and `wrappers/`.

Rationale:
- Keeps shell UX assets versioned alongside the Rust CLIs they accompany.
- Makes local setup reproducible without hopping between repos.
- Enables future automation to generate and update completions in one place.

Location:
- `completions/zsh/`: zsh completion files (generated or curated)
  - `completions/zsh/aliases.zsh`
  - `completions/zsh/_api-rest`
  - `completions/zsh/_api-gql`
  - `completions/zsh/_api-test`
  - `completions/zsh/_git-scope`
  - `completions/zsh/_git-summary`
  - `completions/zsh/_git-lock`
  - `completions/zsh/_fzf-cli`
  - `completions/zsh/_codex-cli`
  - `completions/zsh/_semantic-commit`
  - `completions/zsh/_plan-tooling`
- `completions/bash/`: bash completion files
  - `completions/bash/aliases.bash`
  - `completions/bash/api-rest`
  - `completions/bash/api-gql`
  - `completions/bash/api-test`
  - `completions/bash/git-scope`
  - `completions/bash/git-summary`
  - `completions/bash/git-lock`
  - `completions/bash/fzf-cli`
  - `completions/bash/codex-cli`
  - `completions/bash/semantic-commit`
  - `completions/bash/plan-tooling`
- `wrappers/`: wrapper scripts for invoking CLI binaries or enforcing env setup

Integration steps:
1. Zsh: add `completions/zsh/` to your `fpath`, then run `compinit` in your shell init.
2. Zsh (optional): `source completions/zsh/aliases.zsh`
3. Bash: copy `completions/bash/<command>` into your bash-completion directory, or source them from your shell init.
4. Bash (optional): `source completions/bash/aliases.bash`
5. Dev-only: add `wrappers/` to your PATH (or symlink wrapper scripts into a bin directory).
6. Regenerate completions when CLIs change, and commit updates alongside code.

## License

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

This project is licensed under the MIT License. See [LICENSE](LICENSE).
