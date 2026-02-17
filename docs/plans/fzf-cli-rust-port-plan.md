# Plan: Rust fzf-cli parity (CLI + docs + tests)

## Overview
This plan ports the existing Zsh `fzf-tools` implementation from `https://github.com/graysurf/zsh-kit/blob/main/scripts/fzf-tools.zsh`
into a Rust CLI crate inside this workspace, named `fzf-cli`. The goal is behavioral parity for the
dispatcher commands (help text, guardrails, prompts, and exit codes) while keeping the same external
tooling model (shell out to `fzf`, `git`, `ps`, `lsof`, etc.) and providing deterministic, CI-friendly
tests via stubbed PATH tools.

## Scope
- In scope: Rust `fzf-cli` binary, command dispatcher parity, subcommand behavior parity for
  `file`, `directory`, `git-status`, `git-commit`, `git-checkout`, `git-branch`, `git-tag`,
  `process`, `port`, `history`, `env`, `alias`, `function`, `def`, plus docs (`spec`/`fixtures`)
  and comprehensive edge-case test coverage.
- Out of scope: re-implementing `fzf` itself, perfect preview UX parity across all terminals, and
  introspecting live shell-only state that is not accessible to child processes (documented
  limitations and wrapper guidance are in the spec).

## Assumptions (if any)
1. `fzf-cli` shells out to external commands to match the Zsh script behavior: `fzf`, `git`, `ps`,
   `lsof`/`netstat`, `kill`, `vi`, `code`, `pbcopy` (or equivalents).
2. Interactive pickers can be tested by stubbing `fzf` (and other external tools) via `PATH` so tests
   do not require a real TTY UI.
3. For features that inherently require changing the parent shell state (for example `cd` and `eval`)
   the Rust CLI will provide a safe output contract and a small wrapper snippet documented in the
   spec (rather than relying on impossible parent-process mutation).
4. Docs and tests are the source of truth for parity; if an ambiguity exists, prefer matching the
   current `fzf-tools.zsh` behavior.

## Sprint 1: Parity spec + fixture capture
**Goal**: Make current `fzf-tools` behavior explicit and define test fixtures for parity and edge cases.
**Demo/Validation**:
- Command(s): `rg -n "^# fzf-cli parity spec" crates/fzf-cli/README.md`
- Verify: spec and fixtures describe commands, flags, prompts, errors, and non-trivial edge cases.

### Task 1.1: Write fzf-cli parity spec
- **Location**:
  - `crates/fzf-cli/README.md`
- **Description**: Read `https://github.com/graysurf/zsh-kit/blob/main/scripts/fzf-tools.zsh` and document `fzf-cli` commands,
  argument parsing, output/prompt strings, exit codes, and limitations where parent-shell mutation is
  not possible.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Spec lists all dispatcher commands and their purpose.
  - Spec documents flag parsing for `--vi/--vscode`, `-k/--kill`, and `-9/--force`.
  - Spec documents error messages and exit codes for unknown commands and invalid flags.
  - Spec documents how `directory` and `history` behave in Rust (and how to wrap for shell parity).
- **Validation**:
  - `rg -n "^# fzf-cli parity spec" crates/fzf-cli/README.md`

### Task 1.2: Define fzf-cli fixtures and edge-case matrix
- **Location**:
  - `crates/fzf-cli/README.md`
- **Description**: Define canonical fixture scenarios and edge cases (missing tools, non-git repo,
  flag parse errors, empty selections, kill confirmations, snapshot extraction failures) with setup
  steps and expected output markers.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Fixtures cover every dispatcher command at least once.
  - Fixtures include explicit edge cases for parse errors and missing dependencies.
- **Validation**:
  - `rg -n "^# fzf-cli fixtures" crates/fzf-cli/README.md`

## Sprint 2: Rust crate scaffold + CLI surface
**Goal**: Add the `fzf-cli` crate and implement help/dispatch behavior matching `fzf-tools`.
**Demo/Validation**:
- Command(s): `cargo run -p fzf-cli -- --help`
- Verify: help output lists commands and unknown command behavior matches spec.

### Task 2.1: Create fzf-cli crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/fzf-cli/Cargo.toml`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Add a new Rust binary crate named `fzf-cli`, register it as a workspace member,
  and wire a minimal `main` that dispatches subcommands.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo run -p fzf-cli -- --help` succeeds.
  - Workspace metadata includes `fzf-cli`.
- **Validation**:
  - `cargo metadata --no-deps | rg "\"name\": \"fzf-cli\""`

### Task 2.2: Implement help output + unknown-command guardrails
- **Location**:
  - `crates/fzf-cli/src/main.rs`
- **Description**: Implement `help` output and unknown command messaging mirroring `fzf-tools`,
  including exit codes for help (0) and unknown commands (1).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Empty args and `help/--help/-h` print usage and exit 0.
  - Unknown command prints the `Unknown command` line plus `Run 'fzf-cli help' for usage.` and exits 1.
- **Validation**:
  - `cargo run -p fzf-cli -- help | rg "Usage: fzf-cli"`

## Sprint 3: Shared helpers (fzf runner, prompts, parsing)
**Goal**: Implement reusable building blocks for the interactive commands with testability hooks.
**Demo/Validation**:
- Command(s): `cargo test -p fzf-cli`
- Verify: helpers are unit-tested and integration-friendly (PATH-stubbable).

### Task 3.1: Implement fzf invocation + output parsing
- **Location**:
  - `crates/fzf-cli/src/fzf.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Add a small `fzf` wrapper that feeds stdin, captures stdout, and supports `--expect`
  and `--print-query` result parsing patterns used by multiple subcommands.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Can run `fzf` with provided list input and return selected lines deterministically.
  - Errors bubble up with context indicating which external command failed.
- **Validation**:
  - `cargo test -p fzf-cli fzf_output_parsing`

### Task 3.2: Implement confirmation prompts and kill-flow logic
- **Location**:
  - `crates/fzf-cli/src/confirm.rs`
  - `crates/fzf-cli/src/kill.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `_fzf_confirm`, kill-flag parsing (`-k/--kill`, `-9/--force`), and the shared
  kill flow (prompting and SIGTERM vs SIGKILL dispatch).
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Declining confirmation prints `🚫 Aborted.` and exits non-zero.
  - `-k` performs immediate kill without prompts (SIGTERM by default).
  - `-k -9` performs immediate SIGKILL and prints the corresponding status line.
- **Validation**:
  - `cargo test -p fzf-cli kill_flow`

## Sprint 4: File, directory, history commands
**Goal**: Implement file picking and history selection flows with documented shell-interop behavior.
**Demo/Validation**:
- Command(s): `cargo run -p fzf-cli -- file --vi`
- Verify: commands run end-to-end with stubbed `fzf` in tests; real usage requires `fzf`.

### Task 4.1: Implement file command (open-with parsing and opener)
- **Location**:
  - `crates/fzf-cli/src/file.rs`
  - `crates/fzf-cli/src/open.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-file` behavior: parse `--vi/--vscode`, select a file, and open it with
  `vi` or VSCode (`code --goto`), including workspace-root detection via `.git` scanning.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Unknown `--flag` returns exit code 2 and prints an error to stderr.
  - `--vi` and `--vscode` together return exit code 2 and print the mutual-exclusion error.
  - When VSCode open fails, prints fallback message and opens with `vi`.
- **Validation**:
  - `cargo test -p fzf-cli file_open_with_flags`

### Task 4.2: Implement directory command (two-step picker)
- **Location**:
  - `crates/fzf-cli/src/directory.rs`
  - `crates/fzf-cli/src/main.rs`
  - `crates/fzf-cli/README.md`
- **Description**: Port the two-step `fzf-directory` picker. When the user chooses the cd action,
  print a shell command on stdout suitable for `eval` in the parent shell (documented in the spec).
- **Dependencies**:
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Step 1 preserves query when re-entering directory picker.
  - Step 2 supports open-file action and cd action (emits the shell command for cd).
  - Behavior is explicitly documented in the spec under limitations/wrappers.
- **Validation**:
  - `cargo test -p fzf-cli directory_two_step`

### Task 4.3: Implement history command (Zsh history parsing and selection)
- **Location**:
  - `crates/fzf-cli/src/history.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-history-select` parsing of extended Zsh history (`: epoch:...;cmd`)
  with filtering rules, and implement `history` to print the selected command on stdout for the
  parent shell to evaluate.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Commands with only whitespace/punctuation/control characters are filtered out.
  - Selected command output strips leading icon prefixes matching the Zsh script behavior.
  - Empty selection exits 1 without side effects.
- **Validation**:
  - `cargo test -p fzf-cli history_parsing`

## Sprint 5: Process and port commands
**Goal**: Implement process/port pickers and safe kill behavior with stubbable external commands.
**Demo/Validation**:
- Command(s): `cargo run -p fzf-cli -- process`
- Verify: kill prompts and immediate kill flags behave as documented.

### Task 5.1: Implement process command
- **Location**:
  - `crates/fzf-cli/src/process.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-process` selection and PID extraction. Support multi-select and dispatch
  to the shared kill flow.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Multi-select PID extraction deduplicates and ignores empty selections.
  - `-k/--kill` and `-9/--force` behave as specified for immediate kill.
- **Validation**:
  - `cargo test -p fzf-cli process_kill_flags`

### Task 5.2: Implement port command with lsof and netstat fallback
- **Location**:
  - `crates/fzf-cli/src/port.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-port` selection and PID extraction from `lsof`. When `lsof` is missing,
  fall back to `netstat` in view-only mode (no kill dispatch) and document the behavior in the spec.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - With `lsof` available, selected PIDs are deduplicated and passed to kill flow.
  - Without `lsof`, the command runs and exits 0 after selection without killing processes.
- **Validation**:
  - `cargo test -p fzf-cli port_fallback`

## Sprint 6: Git pickers (status, checkout, branch, tag, commit)
**Goal**: Port the Git-centric interactive workflows using the existing `git-scope` binary for previews.
**Demo/Validation**:
- Command(s): `cargo run -p fzf-cli -- git-checkout`
- Verify: guardrails for non-git repos, confirmations, and git command failures match spec.

### Task 6.1: Implement git-status command
- **Location**:
  - `crates/fzf-cli/src/git_status.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-git-status`: list `git status -s` lines, run `fzf`, and provide a preview
  command that shows staged/unstaged diffs and an untracked diff fallback.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Outside a Git repo prints the same abort message and exits non-zero.
  - Selected file path parsing handles rename `old -> new` and quoted paths.
- **Validation**:
  - `cargo test -p fzf-cli git_status_path_parsing`

### Task 6.2: Implement commit selector helper
- **Location**:
  - `crates/fzf-cli/src/git_commit_select.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Implement a reusable commit picker based on `git log` output and `fzf` selection,
  returning the chosen short hash for downstream commands.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Empty selection exits 1.
  - Output parsing extracts the hash from the first column of the selected line.
- **Validation**:
  - `cargo test -p fzf-cli commit_select`

### Task 6.3: Implement git-checkout command with optional auto-stash retry
- **Location**:
  - `crates/fzf-cli/src/git_checkout.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-git-checkout`: select a commit, confirm checkout, attempt `git checkout`,
  and on failure offer a stash-and-retry flow with a deterministic stash message format.
- **Dependencies**:
  - Task 6.2
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Declining confirmations prints `🚫 Aborted.` and exits non-zero.
  - On checkout failure, prompts for stash and retries checkout when confirmed.
- **Validation**:
  - `cargo test -p fzf-cli git_checkout_stash_flow`

### Task 6.4: Implement git-branch and git-tag commands
- **Location**:
  - `crates/fzf-cli/src/git_branch.rs`
  - `crates/fzf-cli/src/git_tag.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Port `fzf-git-branch` and `fzf-git-tag` browse-and-checkout flows with confirmation
  prompts and matching success/warning output.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Non-git repo prints the abort message and exits 1.
  - Checkout success prints the same success line format.
  - Tag resolution failures print the same error and exit non-zero.
- **Validation**:
  - `cargo test -p fzf-cli git_branch_tag`

### Task 6.5: Implement git-commit command (open worktree files or snapshots)
- **Location**:
  - `crates/fzf-cli/src/git_commit.rs`
  - `crates/fzf-cli/src/main.rs`
  - `crates/fzf-cli/README.md`
- **Description**: Port `fzf-git-commit` including `--snapshot` behavior, file list rendering with
  per-file stats, and snapshot extraction to a temporary file with cleanup. Ensure editor behavior
  (`vi` vs `vscode`) matches the script as closely as possible and document deviations.
- **Dependencies**:
  - Task 6.2
  - Task 4.1
  - Task 3.2
- **Complexity**: 9
- **Acceptance criteria**:
  - When no worktree files exist for a commit, prints the matching error and re-prompts commit select.
  - Snapshot extraction errors print the matching error and exit non-zero.
  - Temporary files are removed after editor exit.
- **Validation**:
  - `cargo test -p fzf-cli git_commit_snapshot`

## Sprint 7: Definition browser commands (env, alias, function, def)
**Goal**: Port the definition browser flows, including docblock indexing and optional cache.
**Demo/Validation**:
- Command(s): `cargo run -p fzf-cli -- def`
- Verify: missing delimiter env variables show the same error text and exit non-zero.

### Task 7.1: Implement first-party zsh definition indexer and optional cache
- **Location**:
  - `crates/fzf-cli/src/defs/index.rs`
  - `crates/fzf-cli/src/defs/cache.rs`
  - `crates/fzf-cli/src/main.rs`
- **Description**: Implement a file-based indexer for first-party zsh files and docblocks (function
  and alias comment blocks), and an optional persistent cache with TTL behavior equivalent to the
  script’s `FZF_DEF_DOC_CACHE_ENABLED` and `FZF_DEF_DOC_CACHE_EXPIRE_MINUTES` settings.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Indexer scans `.zshrc`, `.zprofile`, and zsh files under `scripts/`, `bootstrap/`, `tools/`.
  - Cache respects TTL and avoids re-indexing when fresh.
- **Validation**:
  - `cargo test -p fzf-cli defs_index_and_cache`

### Task 7.2: Implement env/alias/function/def using block preview and clipboard copy
- **Location**:
  - `crates/fzf-cli/src/defs/block_preview.rs`
  - `crates/fzf-cli/src/defs/commands.rs`
  - `crates/fzf-cli/src/main.rs`
  - `crates/fzf-cli/README.md`
- **Description**: Port `fzf_block_preview` behavior: require delimiter env vars, generate blocks,
  run `fzf` with a preview script, print the selected block, and copy it to the clipboard.
- **Dependencies**:
  - Task 7.1
  - Task 3.1
- **Complexity**: 8
- **Acceptance criteria**:
  - When delimiter env vars are missing, prints the same two-line error/help text and exits 1.
  - Selected output is printed and clipboard copy is attempted (best-effort).
  - `def` aggregates env + alias + function blocks.
- **Validation**:
  - `cargo test -p fzf-cli def_block_preview`

## Sprint 8: Comprehensive tests + delivery gates
**Goal**: Add deterministic integration tests covering all commands and edge cases; ensure repo checks pass.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: fmt, clippy, workspace tests, and zsh completion tests all pass.

### Task 8.1: Add fzf-cli test harness (PATH stubs and temp fixtures)
- **Location**:
  - `crates/fzf-cli/tests/common.rs`
- **Description**: Create shared helpers for running the `fzf-cli` binary in tests, including PATH
  stubbing for `fzf`, `vi`, `code`, `git`, and other external commands as needed.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests can inject stub binaries via PATH without affecting the developer machine.
  - Helpers support piping stdin to simulate confirmation prompts.
- **Validation**:
  - `cargo test -p fzf-cli common_helpers`

### Task 8.2: Implement edge-case integration tests for every dispatcher command
- **Location**:
  - `crates/fzf-cli/tests/edge_cases.rs`
  - `crates/fzf-cli/tests/help_and_dispatch.rs`
- **Description**: Add integration tests that assert exit codes and key output strings for all
  dispatcher commands, including parse errors, missing env vars, missing git repo, and kill flows.
- **Dependencies**:
  - Task 8.1
  - Task 4.1
  - Task 4.2
  - Task 4.3
  - Task 5.1
  - Task 5.2
  - Task 6.1
  - Task 6.3
  - Task 6.4
  - Task 6.5
  - Task 7.2
- **Complexity**: 9
- **Acceptance criteria**:
  - All dispatcher commands have at least one passing integration test.
  - Edge-case tests cover non-zero exit codes and matching error strings.
- **Validation**:
  - `cargo test -p fzf-cli`

### Task 8.3: Run full repo delivery checks
- **Location**:
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- **Description**: Run the repo’s required pre-delivery checks and fix failures within the scope of
  the `fzf-cli` work until the full suite passes.
- **Dependencies**:
  - Task 8.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo fmt --all -- --check` passes.
  - `cargo clippy --all-targets --all-features -- -D warnings` passes.
  - `cargo test --workspace` passes.
  - `zsh -f tests/zsh/completion.test.zsh` passes.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Testing Strategy
- Unit: parsing helpers (history parsing, git-status path parsing), def indexer, and fzf output parsing.
- Integration: run the built `fzf-cli` binary with PATH-stubbed tools and temporary git repos.
- E2E/manual: sanity-run interactive commands against a real repo with real `fzf` for UX verification.

## Risks & gotchas
- Parent-shell mutation limitations: `cd` and `eval` cannot affect the caller shell; document wrapper usage.
- External tool variance: `lsof`, `netstat`, `delta`, `bat`, `fd` differ across platforms; prefer graceful
  degradation and cover critical paths with stubs in tests.
- Prompt safety: kill flows must not run destructive actions without explicit confirmation or flags.

## Rollback plan
- Remove the workspace member `crates/fzf-cli` and associated docs/tests.
- Delete any wrapper additions (if any) and re-run `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`.
