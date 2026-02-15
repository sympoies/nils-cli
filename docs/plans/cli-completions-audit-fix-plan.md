# Plan: CLI Completion Audit + Fix

## Overview
Update all Zsh completion scripts under `completions/zsh/` to fully match the Rust CLI surfaces (subcommands, nested commands, and flags). The plan inventories each CLI’s command tree from source/README and corrects completions accordingly, including wrapper aliases where applicable. The goal is consistent, accurate completions across all shipped CLIs, validated by repo checks.

## Scope
- In scope: Zsh completion files in `completions/zsh/` for all shipped CLIs (git-scope, git-summary, git-lock, fzf-cli, semantic-commit, api-rest, api-gql, api-test, plan-tooling, codex-cli).
- Out of scope: Bash/Fish completions, CLI behavior changes, command implementations, docs beyond completion comments.

## Assumptions (if any)
1. Zsh completion is the only supported shell completion in this repo.
2. The authoritative CLI surfaces are in `crates/*/src` and (where present) corresponding README/help text.

## Sprint 1: Full Completion Parity
**Goal**: Every shipped CLI completion file matches the current command tree and flags, including multi-level subcommands and wrapper aliases.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: completion test passes; manual spot-checks on key commands show full subcommand/flag coverage.

### Task 1.1: Inventory command surfaces
- **Location**:
  - `crates/git-scope/src/main.rs`
  - `crates/git-summary/src/main.rs`
  - `crates/git-lock/src/main.rs`
  - `crates/fzf-cli/src/main.rs`
  - `crates/semantic-commit/src/main.rs`
  - `crates/api-rest/src/cli.rs`
  - `crates/api-gql/src/cli.rs`
  - `crates/api-test/src/main.rs`
  - `crates/plan-tooling/src/lib.rs`
  - `crates/codex-cli/src/cli.rs`
- **Description**: Build a command/flag matrix for each CLI with subcommands, nested args, and special flags (including default subcommand behavior for api-rest/api-gql/api-test).
- **Dependencies**: none
- **Complexity**: 2
- **Acceptance criteria**:
  - Each CLI has an explicit list of subcommands and flags mapped to its completion file.
- **Validation**:
  - Cross-check lists with source structs or manual parsing of `main.rs`/`cli.rs`.

### Task 1.2: Fix `git-scope` completion
- **Location**:
  - `completions/zsh/_git-scope`
  - `crates/git-scope/src/main.rs`
- **Description**: Ensure `tracked/staged/unstaged/all/untracked/commit/help` plus flags (`--no-color`, `-p/--print`, `--parent/-P`) and positional behavior are correct.
- **Dependencies**: Task 1.1
- **Complexity**: 1
- **Acceptance criteria**:
  - All git-scope flags and commit parent options are suggested at the right positions.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.3: Fix `git-summary` completion
- **Location**:
  - `completions/zsh/_git-summary`
  - `crates/git-summary/src/main.rs`
- **Description**: Confirm subcommands + custom date range behavior and help flags.
- **Dependencies**: Task 1.1
- **Complexity**: 1
- **Acceptance criteria**:
  - All summary commands appear and positional date prompts align with CLI.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.4: Fix `git-lock` completion
- **Location**:
  - `completions/zsh/_git-lock`
  - `crates/git-lock/src`
- **Description**: Ensure subcommands (`lock/unlock/list/copy/delete/diff/tag/help`) and per-subcommand flags (`diff --no-color`, `tag --push -m`) plus label suggestions are accurate.
- **Dependencies**: Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - Completion lists flags for diff/tag and label positions correctly.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.5: Fix `fzf-cli` completion
- **Location**:
  - `completions/zsh/_fzf-cli`
  - `crates/fzf-cli/src`
- **Description**: Ensure file/directory open flags (`--vi/--vscode`, `--`) and process/port kill flags (`-k/--kill`, `-9/--force`) are complete; confirm all subcommands.
- **Dependencies**: Task 1.1
- **Complexity**: 1
- **Acceptance criteria**:
  - All subcommands and flags appear in completion suggestions.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.6: Fix `semantic-commit` completion
- **Location**:
  - `completions/zsh/_semantic-commit`
  - `crates/semantic-commit/src/main.rs`
- **Description**: Ensure `staged-context/commit/help` plus `commit` flags (`--message`, `--message-file`) are complete.
- **Dependencies**: Task 1.1
- **Complexity**: 1
- **Acceptance criteria**:
  - `commit` completion lists both message options and help.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.7: Fix `api-rest` completion
- **Location**:
  - `completions/zsh/_api-rest`
  - `crates/api-rest/src/cli.rs`
- **Description**: Ensure default `call` behavior, subcommands, and all flags (`call/history/report/report-from-cmd`) align with clap definitions.
- **Dependencies**: Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - All flags from `CallArgs`, `HistoryArgs`, `ReportArgs`, `ReportFromCmdArgs` are in completions.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.8: Fix `api-gql` completion
- **Location**:
  - `completions/zsh/_api-gql`
  - `crates/api-gql/src/cli.rs`
- **Description**: Ensure default `call` behavior, subcommands, and all flags (`call/history/report/report-from-cmd/schema`) align with clap definitions.
- **Dependencies**: Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - All flags from `CallArgs`, `HistoryArgs`, `ReportArgs`, `ReportFromCmdArgs`, `SchemaArgs` are in completions.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.9: Fix `api-test` completion
- **Location**:
  - `completions/zsh/_api-test`
  - `crates/api-test/src/main.rs`
- **Description**: Ensure default `run` behavior, `run`/`summary` flags, and help/version flags match clap definitions.
- **Dependencies**: Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - All flags from `RunArgs` and `SummaryArgs` are suggested correctly.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.10: Fix `plan-tooling` completion
- **Location**:
  - `completions/zsh/_plan-tooling`
  - `crates/plan-tooling/src/lib.rs`
- **Description**: Ensure subcommands and flags align with plan-tooling CLI (`to-json/validate/batches/scaffold/help`).
- **Dependencies**: Task 1.1
- **Complexity**: 1
- **Acceptance criteria**:
  - All plan-tooling flags and help options appear.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 1.11: Fix `codex-cli` completion
- **Location**:
  - `completions/zsh/_codex-cli`
  - `crates/codex-cli/src/cli.rs`
- **Description**: Ensure nested subcommands (`agent/auth/diag/config/starship`) and their flags, plus wrapper aliases and help/version flags.
- **Dependencies**: Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - All nested commands and flags are available in completion, including wrappers.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

## Testing Strategy
- Unit: none (completion scripts only).
- Integration: `zsh -f tests/zsh/completion.test.zsh`.
- E2E/manual: Spot-check `codex-cli`, `api-rest`, `api-gql`, `api-test` with nested subcommands and flag suggestions.

## Risks & gotchas
- CLI flags may differ subtly from README/help; prioritize source (`cli.rs`/`main.rs`) over docs.
- Some CLIs use default subcommand behavior (api-rest/api-gql/api-test), which can confuse completion ordering; ensure top-level completion handles both explicit and implicit commands.

## Rollback plan
- Revert changes to the modified completion files under `completions/zsh/`.
