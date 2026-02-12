# nils-cli

[![CI](https://github.com/graysurf/nils-cli/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/graysurf/nils-cli/actions/workflows/ci.yml) [![Coverage](https://raw.githubusercontent.com/graysurf/nils-cli/coverage-badge/badges/coverage.svg)](https://github.com/graysurf/nils-cli/actions/workflows/ci.yml) [![Release](https://img.shields.io/github/v/release/graysurf/nils-cli?sort=semver)](https://github.com/graysurf/nils-cli/releases) [![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A Rust workspace of focused CLI binaries for Git operations, API test orchestration, and workflow automation, unified by shared cross-crate helpers.

## Workspace layout

Each crate is either a standalone CLI binary or a shared library used across the workspace.

### Shared foundations

- [crates/nils-common](crates/nils-common): Shared cross-CLI utilities (small helpers that don’t belong to a specific binary).
- [crates/nils-term](crates/nils-term): Terminal UX helpers (TTY detection + progress rendering on stderr).
- [crates/nils-test-support](crates/nils-test-support): Test-only helpers for deterministic workspace integration tests.
- [crates/cli-template](crates/cli-template): Minimal example CLI for validating packaging and new-crate patterns.

### API testing stack

- [crates/api-testing-core](crates/api-testing-core): Shared library for the API testing CLIs (config/auth, history, reporting).
- [crates/api-rest](crates/api-rest): REST request runner from JSON request specs, with history + Markdown reports.
- [crates/api-gql](crates/api-gql): GraphQL operation runner for `.graphql` files (variables, history, reports, schema).
- [crates/api-test](crates/api-test): Suite runner that orchestrates REST/GraphQL cases and outputs JSON (and optional JUnit).

### Git tooling

- [crates/git-scope](crates/git-scope): Git change inspector (tracked/staged/unstaged/untracked/commit) with tree + optional file printing.
- [crates/git-cli](crates/git-cli): Git tools dispatcher (utils/reset/commit/branch/ci).
- [crates/git-summary](crates/git-summary): Per-author contribution summaries over a date range (adds/dels/net/commits).
- [crates/git-lock](crates/git-lock): Label-based commit locks per repo (lock/list/diff/unlock/tag).

### Automation and utility CLIs

- [crates/macos-agent](crates/macos-agent): macOS desktop automation primitives for app/window discovery, input actions, screenshot, and wait helpers.
- [crates/fzf-cli](crates/fzf-cli): Interactive `fzf` toolbox for files, Git, processes, ports, and shell history.
- [crates/memo-cli](crates/memo-cli): Capture-first memo workflow CLI with agent enrichment loop (`add`, `list`, `search`, `report`, `fetch`, `apply`).
- [crates/image-processing](crates/image-processing): Batch image transformation CLI (resize/crop/optimize) with JSON/report outputs.
- [crates/screen-record](crates/screen-record): macOS ScreenCaptureKit + Linux (X11) recorder for a single window or display with optional audio.

### Agent and workflow tooling

- [crates/agent-docs](crates/agent-docs): Deterministic policy-document resolver for Codex/agent workflows (`resolve`, `contexts`, `add`, `baseline`).
- [crates/agent-runtime-core](crates/agent-runtime-core): Provider-neutral runtime contract (`provider-adapter.v1`) shared by `agentctl` and provider adapters.
- [crates/agent-provider-codex](crates/agent-provider-codex): Stable OpenAI/Codex adapter implementation for the provider-neutral contract.
- [crates/agent-provider-claude](crates/agent-provider-claude): Compile-only `claude` onboarding stub adapter (`maturity=stub`).
- [crates/agent-provider-gemini](crates/agent-provider-gemini): Compile-only `gemini` onboarding stub adapter (`maturity=stub`).
- [crates/agentctl](crates/agentctl): Provider-neutral control plane (`provider`, `diag`, `debug`, `workflow`, `automation`).
- [crates/codex-cli](crates/codex-cli): Provider-specific CLI for OpenAI/Codex workflows (auth, Codex diagnostics, Codex execution wrappers, Starship snippets).
- [crates/semantic-commit](crates/semantic-commit): Helper CLI for generating staged context and creating semantic commits.
- [crates/plan-tooling](crates/plan-tooling): Plan Format v1 tooling CLI (to-json/validate/batches/scaffold).

## Shared helper policy (`nils-common`)

Contributors should treat `nils-common` as the shared helper boundary for cross-CLI primitives.

- Put reusable, domain-neutral helpers in [crates/nils-common](crates/nils-common).
- Keep crate-local adapters for user-facing copy, warning style, exit-code mapping, and CLI-specific UX policy.
- During migration, preserve parity by keeping output text/warnings/colors/exit behavior byte-for-byte stable.
- Characterize behavior with tests before moving helper logic, then re-run affected crate tests after migration.

Detailed scope, API examples, migration conventions, and non-goals are documented in
[crates/nils-common/README.md](crates/nils-common/README.md).

## Shell wrappers and completions

This repo keeps optional wrapper scripts and completion assets in-repo.

Location:

- [completions/zsh/](completions/zsh/): zsh completions (plus `aliases.zsh`)
- [completions/bash/](completions/bash/): bash completions (plus `aliases.bash`)
- [wrappers/](wrappers/): dev-only wrapper scripts

Integration steps:

1. Zsh: add [completions/zsh/](completions/zsh/) to your `fpath`, then run `compinit` in your shell init.
2. Zsh (optional): `source completions/zsh/aliases.zsh` (see [completions/zsh/aliases.zsh](completions/zsh/aliases.zsh))
3. Bash: copy `completions/bash/<command>` into your bash-completion directory, or source them from your shell init.
4. Bash (optional): `source completions/bash/aliases.bash` (see [completions/bash/aliases.bash](completions/bash/aliases.bash))
5. Dev-only: add [wrappers/](wrappers/) to your PATH (or symlink wrapper scripts into a bin directory).
6. Regenerate completions when CLIs change, and commit updates alongside code.

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

To trigger a release build, push a tag like `v0.3.4`:

- `git tag -a v0.3.4 -m "v0.3.4"`
- `git push origin v0.3.4`

Then download the matching `nils-cli-<tag>-<target>.tar.gz` asset, extract it, and add
`<extract_dir>/bin` to your `PATH`.

For zsh completions, add `<extract_dir>/completions/zsh` to your `fpath` and run `compinit`.
Optional: source `<extract_dir>/completions/zsh/aliases.zsh` to enable `gs*`/`cx*`/`fx*` aliases.

For bash completions, copy `<extract_dir>/completions/bash/<command>` into your bash-completion directory
(example: `~/.local/share/bash-completion/completions/`) or source it from your shell init.
Optional: source `<extract_dir>/completions/bash/aliases.bash` to enable `gs*`/`cx*`/`fx*` aliases.

## crates.io publishing (shared crates)

Use `scripts/publish-crates.sh` for crate publishing flow.

- Default list + order: `release/crates-io-publish-order.txt`
- Dry-run (recommended first):
  - `scripts/publish-crates.sh --dry-run`
- Real publish:
  - `scripts/publish-crates.sh --publish`
- Override target crates:
  - `scripts/publish-crates.sh --crates "nils-term nils-common" --dry-run`

In `--dry-run` mode, the script runs `cargo publish --dry-run` for every selected crate.
In `--publish` mode, the script runs `dry-run -> publish` sequentially per crate (in your specified order).
By default, `--publish` skips crates that are already published at the same version on crates.io.

To query crates.io publish status (single/multi/all crates), use:

- `scripts/crates-io-status.sh --all --format text`
- `scripts/crates-io-status.sh --crates "nils-common nils-codex-cli" --version v0.3.1 --format json`
- `scripts/crates-io-status.sh --crate nils-codex-cli --format both --json-out "$CODEX_HOME/out/codex-status.json"`

`--version` checks that exact version; without `--version` the script checks each crate's current workspace version.
Use `--fail-on-missing` for CI gates.

GitHub Actions manual flow is also available at `.github/workflows/publish-crates.yml`:

- Trigger via `workflow_dispatch`.
- `mode=dry-run` runs checks only.
- `mode=publish` requires repository secret `CARGO_REGISTRY_TOKEN`.

## Adding a new CLI crate

1. Create a new binary crate under [crates/](crates/):
   - `cargo new crates/<cli-name> --bin`
2. Add the crate path to the workspace `members` list in [Cargo.toml](Cargo.toml).
3. Use shared dependencies via `workspace = true` in the new crate's [Cargo.toml](Cargo.toml).
4. Build or run the new CLI with `cargo build -p <cli-name>` or `cargo run -p <cli-name> -- ...`.
5. Verify packaging picks it up (both local install + GitHub Releases use the same discovery):
   - `python3 scripts/workspace-bins.py | rg "^<cli-name>$"` (see [scripts/workspace-bins.py](scripts/workspace-bins.py))
