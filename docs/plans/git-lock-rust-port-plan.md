# Plan: Rust git-lock parity (CLI + completion + tests)

## Overview
This plan ports the existing Zsh `git-lock` helper into a Rust CLI crate inside this workspace,
matching behavior, output text, and prompts from `https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-lock.zsh`. It also
records upstream reference links for repeatable parity reference, ports the Zsh completion script,
adds wrapper scripts, and delivers a comprehensive Rust integration test suite (including edge
cases like missing labels, invalid commits, and confirmation prompts). The outcome is a `git-lock`
binary with matching UX and a repeatable test suite that validates parity across commands.

## Scope
- In scope: Rust `git-lock` CLI implementation, output parity with the Zsh script, upstream source references
  for parity reference, zsh completion port, wrapper script, and a full test suite covering
  commands and edge cases.
- Out of scope: New subcommands, alternative lock storage locations, or behavior changes beyond
  parity with the current Zsh implementation.

## Assumptions (if any)
1. The Rust CLI shells out to `git` for repository data and mutations.
2. Lock files remain under `$ZSH_CACHE_DIR/git-locks` with the same file format as the script.
3. The CLI keeps the same prompt strings and emojis as the Zsh implementation.
4. Zsh completion file will live at `completions/zsh/_git-lock` and register `git-lock`.
5. Tests can create temporary git repos and simulate stdin for confirmation prompts.

## Sprint 1: Parity spec + fixtures
**Goal**: Make current git-lock behavior explicit and capture fixtures for parity.
**Demo/Validation**:
- Command(s): `rg -n "git-lock" crates/git-lock/README.md`, `rg -n "compdef" completions/zsh/_git-lock`
- Verify: Spec doc includes commands, flags, file format, and edge-case behavior.
**Parallelizable**: none.

### Task 1.1: Capture upstream Zsh references into repo docs
- **Location**:
  - `crates/git-lock/README.md`
  - `https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-lock.zsh`
  - `https://github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-lock`
  - `https://github.com/graysurf/zsh-kit/blob/main/docs/cli/git-lock.md`
- **Description**: Record upstream script/completion/doc references as GitHub links so parity
  sources are stable and not tied to a local filesystem snapshot.
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - Repo docs include upstream links for the script, completion, and doc.
- **Validation**:
  - `rg "github.com/graysurf/zsh-kit" crates/git-lock/README.md`

### Task 1.2: Document current git-lock behavior and output contract
- **Location**:
  - `crates/git-lock/README.md`
- **Description**: Read the Zsh implementation and docs to produce a concise spec covering
  commands, flags, output sections, confirmation prompts, lock file format, and error handling.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Spec lists commands and per-command usage notes.
  - Spec captures confirmation prompts and error messages.
  - Spec documents lock file format and latest marker behavior.
- **Validation**:
  - `rg "Commands" crates/git-lock/README.md`
  - `rg "latest" crates/git-lock/README.md`

### Task 1.3: Capture fixture scenarios for tests
- **Location**:
  - `crates/git-lock/README.md`
- **Description**: Define canonical test scenarios (lock/unlock/list/copy/delete/diff/tag,
  missing labels, invalid commit, overwrite prompts, latest label resolution, and not-a-repo)
  with setup steps and expected output markers.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Fixtures list covers all commands and edge cases.
  - Each fixture includes setup steps and expected output markers.
- **Validation**:
  - `rg "^##" crates/git-lock/README.md`
  - `rg "unlock" crates/git-lock/README.md`

## Sprint 2: Rust crate scaffold + CLI surface
**Goal**: Add a new `git-lock` crate and CLI interface matching the script.
**Demo/Validation**:
- Command(s): `cargo metadata --no-deps | rg "git-lock"`, `cargo run -p git-lock -- --help`
- Verify: CLI help lists subcommands and usage matching the spec.
**Parallelizable**: none.

### Task 2.1: Create `git-lock` binary crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/git-lock/Cargo.toml`
  - `crates/git-lock/src/main.rs`
- **Description**: Add a new Rust binary crate named `git-lock` and register it as a workspace member.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace metadata lists `git-lock`.
  - `cargo run -p git-lock -- --help` succeeds.
- **Validation**:
  - `cargo metadata --no-deps | rg "git-lock"`
  - `cargo run -p git-lock -- --help`

### Task 2.2: Implement CLI parsing with clap
- **Location**:
  - `crates/git-lock/src/main.rs`
- **Description**: Implement subcommands (`lock`, `unlock`, `list`, `copy`, `delete`, `diff`, `tag`)
  and options (`--no-color` for diff, `-m`/`--push` for tag, `-h/--help`).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Help output lists all commands and usage strings matching the script.
  - Unknown command prints the same error text as the script.
- **Validation**:
  - `cargo run -p git-lock -- --help | rg "lock"`
  - `cargo run -p git-lock -- diff --help | rg "no-color"`

## Sprint 3: Core lock storage + commands
**Goal**: Implement lock storage, latest marker handling, and core commands.
**Demo/Validation**:
- Command(s): `rg -n "git-lock" crates/git-lock/src`, `cargo run -p git-lock -- lock`
- Verify: Lock files are created and list output matches format.
**Parallelizable**: After Task 3.1, Tasks 3.2–3.4 can run in parallel.

### Task 3.1: Implement repo ID + lock directory helpers
- **Location**:
  - `crates/git-lock/src/fs.rs`
  - `crates/git-lock/src/main.rs`
- **Description**: Add helpers to resolve repo ID, lock directory, lock file paths, and latest marker.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Repo ID matches `basename(git rev-parse --show-toplevel)`.
  - Lock directory is `$ZSH_CACHE_DIR/git-locks` and created when needed.
- **Validation**:
  - `rg "ZSH_CACHE_DIR" crates/git-lock/src/fs.rs`
  - `rg "rev-parse --show-toplevel" crates/git-lock/src/fs.rs`

### Task 3.2: Implement `lock` command
- **Location**:
  - `crates/git-lock/src/lock.rs`
- **Description**: Save commit hash with optional note and timestamp, write lock file + latest marker,
  and render success output.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Default label is `default`, default commit is `HEAD`.
  - Invalid commit prints `❌ Invalid commit:` followed by the commit value.
  - Output matches script lines and timestamp format.
- **Validation**:
  - `rg "Invalid commit" crates/git-lock/src/lock.rs`
  - `ZSH_CACHE_DIR="$(mktemp -d)" cargo run -p git-lock -- lock`

### Task 3.3: Implement `unlock` command
- **Location**:
  - `crates/git-lock/src/unlock.rs`
- **Description**: Port unlock flow with confirmation prompt and reset, matching script messages.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Missing label uses latest marker; missing latest prints the same error message.
  - Confirmation prompt aborts with `🚫 Aborted` when declined.
- **Validation**:
  - `rg "Hard reset" crates/git-lock/src/unlock.rs`
  - `rg "Aborted" crates/git-lock/src/unlock.rs`

### Task 3.4: Implement `list` command
- **Location**:
  - `crates/git-lock/src/list.rs`
- **Description**: Port list output with timestamp sorting and latest marker display.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - List sorts newest-first by timestamp and marks latest with ⭐.
  - Missing lock directory prints the same “no git-locks found” message.
- **Validation**:
  - `rg "git-lock list" crates/git-lock/src/list.rs`
  - `rg "latest" crates/git-lock/src/list.rs`

## Sprint 4: Copy / delete / diff parity
**Goal**: Port copy, delete, and diff behaviors with parity output.
**Demo/Validation**:
- Command(s): `rg -n "copy|delete|diff" crates/git-lock/src`, `cargo run -p git-lock -- diff --help`
- Verify: Commands match usage/error handling and output format.
**Parallelizable**: After Task 3.1, Tasks 4.1–4.3 can run in parallel.

### Task 4.1: Implement `copy` command
- **Location**:
  - `crates/git-lock/src/copy.rs`
- **Description**: Port label resolution, overwrite prompts, latest marker updates, and output
  formatting for copy.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Copy uses latest when source label omitted and updates latest marker to target.
  - Prompt mirrors the script and aborts with `🚫 Aborted` when declined.
- **Validation**:
  - `rg "Copied git-lock" crates/git-lock/src/copy.rs`
  - `rg "Overwrite" crates/git-lock/src/copy.rs`

### Task 4.2: Implement `delete` command
- **Location**:
  - `crates/git-lock/src/delete.rs`
- **Description**: Port delete flow with confirmation prompt and latest marker cleanup.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Delete removes latest marker if the deleted label was latest.
  - Prompt mirrors the script and aborts with `🚫 Aborted` when declined.
- **Validation**:
  - `rg "Deleted git-lock" crates/git-lock/src/delete.rs`
  - `rg "Removed latest" crates/git-lock/src/delete.rs`

### Task 4.3: Implement `diff` command
- **Location**:
  - `crates/git-lock/src/diff.rs`
- **Description**: Port label resolution, usage errors, and git log rendering with optional
  no-color behavior.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Too many labels prints the usage warning.
  - `--no-color` or `NO_COLOR` passes `--color=never` to git log.
  - Header lines match the script output.
- **Validation**:
  - `rg "Comparing commits" crates/git-lock/src/diff.rs`
  - `rg "color=never" crates/git-lock/src/diff.rs`

## Sprint 5: Tag command parity
**Goal**: Port tag creation workflow with overwrite prompts and push behavior.
**Demo/Validation**:
- Command(s): `rg -n "tag" crates/git-lock/src/tag.rs`, `cargo run -p git-lock -- tag --help`
- Verify: Tag creation output and overwrite confirmation match the script.
**Parallelizable**: After Task 3.1, Task 5.1 can run in parallel with Sprint 4 tasks.

### Task 5.1: Implement `tag` command
- **Location**:
  - `crates/git-lock/src/tag.rs`
- **Description**: Port annotated tag creation, default message selection, overwrite prompt, and
  optional `--push` behavior.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Existing tag prompts for overwrite and deletes before re-tagging.
  - Default tag message is commit subject when `-m` omitted.
  - `--push` pushes to origin and removes local tag afterward.
- **Validation**:
  - `rg "Created tag" crates/git-lock/src/tag.rs`
  - `rg "push" crates/git-lock/src/tag.rs`

## Sprint 6: Zsh completion + wrappers
**Goal**: Ship zsh completion and wrapper scripts aligned with current usage.
**Demo/Validation**:
- Command(s): `rg "git-lock" completions/zsh/_git-lock`, `rg "git-lock" wrappers/git-lock`
- Verify: Completion and wrapper scripts match existing patterns.
**Parallelizable**: Tasks 6.1 and 6.2 can run in parallel.

### Task 6.1: Port zsh completion script
- **Location**:
  - `completions/zsh/_git-lock`
- **Description**: Port `https://github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-lock` into this repo, preserving
  subcommands, options, and diff/tag completion behavior.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Completion file registers for `git-lock`.
  - Completion lists commands and flags from the script.
- **Validation**:
  - `rg "compdef _git-lock git-lock" completions/zsh/_git-lock`
  - `rg "diff" completions/zsh/_git-lock`

### Task 6.2: Add wrapper script for git-lock
- **Location**:
  - `wrappers/git-lock`
- **Description**: Provide a wrapper that invokes the Rust binary or falls back to cargo run.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 2
- **Acceptance criteria**:
  - Wrapper mirrors the existing style of `wrappers/git-scope`.
- **Validation**:
  - `rg "git-lock" wrappers/git-lock`

## Sprint 7: Tests + docs + validation pass
**Goal**: Add comprehensive tests (including edge cases) and update docs.
**Demo/Validation**:
- Command(s): `cargo test -p git-lock`, `zsh -f tests/zsh/completion.test.zsh`
- Verify: Tests cover edge cases and docs describe usage.
**Parallelizable**: Tasks 7.1–7.5 can run in parallel once dependencies are met.

### Task 7.1: Add lock/unlock tests + shared helpers
- **Location**:
  - `crates/git-lock/tests/common.rs`
  - `crates/git-lock/tests/lock_unlock.rs`
- **Description**: Create integration tests for lock/unlock behavior, with temp repos and simulated
  stdin for confirmation prompts.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests set `ZSH_CACHE_DIR` and assert lock/unlock output markers.
  - Cancelled unlock prints `🚫 Aborted` and avoids reset.
- **Validation**:
  - `cargo test -p git-lock --test lock_unlock`

### Task 7.2: Add list tests
- **Location**:
  - `crates/git-lock/tests/list.rs`
- **Description**: Add tests for list sorting, latest marker display, and no-locks output.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests verify newest-first ordering and ⭐ latest marker.
- **Validation**:
  - `cargo test -p git-lock --test list`

### Task 7.3: Add copy/delete tests
- **Location**:
  - `crates/git-lock/tests/copy_delete.rs`
- **Description**: Add tests for copy overwrite prompts, latest marker updates, and delete cleanup.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Tests exercise overwrite prompts and aborted deletions.
- **Validation**:
  - `cargo test -p git-lock --test copy_delete`

### Task 7.4: Add diff/tag tests
- **Location**:
  - `crates/git-lock/tests/diff_tag.rs`
- **Description**: Add tests for diff no-color behavior, header output, and tag overwrite prompts.
- **Dependencies**:
  - Task 4.3
  - Task 5.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests verify `--no-color` passes `--color=never` to git log.
  - Tag overwrite prompts are exercised with simulated stdin.
- **Validation**:
  - `cargo test -p git-lock --test diff_tag`

### Task 7.5: Add edge case tests
- **Location**:
  - `crates/git-lock/tests/edge_cases.rs`
- **Description**: Add tests for not-a-repo handling, invalid commits, and missing latest label.
- **Dependencies**:
  - Task 2.2
  - Task 3.2
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Edge-case tests assert error messages match the script.
- **Validation**:
  - `cargo test -p git-lock --test edge_cases`

### Task 7.6: Update completion smoke test
- **Location**:
  - `tests/zsh/completion.test.zsh`
- **Description**: Extend the zsh completion smoke test to validate `_git-lock` loading and
  command list presence.
- **Dependencies**:
  - Task 6.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Test asserts `_git-lock` function exists and contains command entries.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 7.7: Update README + completion strategy docs
- **Location**:
  - `README.md`
- **Description**: Document the `git-lock` crate, usage examples, and completion file location.
- **Dependencies**:
  - Task 6.2
  - Task 6.1
- **Complexity**: 3
- **Acceptance criteria**:
  - README lists `crates/git-lock` and example commands.
  - README references `_git-lock`.
- **Validation**:
  - `rg "git-lock" README.md`
  - `rg "_git-lock" README.md`

### Task 7.8: End-to-end validation
- **Location**:
  - `crates/git-lock`
  - `tests/zsh`
- **Description**: Run formatting, linting, and tests required by the repo development guide,
  plus git-lock coverage to satisfy the request for full test coverage.
- **Dependencies**:
  - Task 7.1
  - Task 7.2
  - Task 7.3
  - Task 7.4
  - Task 7.5
  - Task 7.6
  - Task 7.7
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo fmt --all -- --check` passes.
  - `cargo clippy --all-targets --all-features -- -D warnings` passes.
  - `cargo test -p nils-common`, `cargo test -p git-scope`, `cargo test -p git-lock` pass.
  - `zsh -f tests/zsh/completion.test.zsh` passes.
  - (Extra) `cargo test -p git-summary` also passes to honor the “all tests” request.
- **Validation**:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test -p nils-common`
  - `cargo test -p git-scope`
  - `cargo test -p git-summary`
  - `cargo test -p git-lock`
  - `zsh -f tests/zsh/completion.test.zsh`

## Testing Strategy
- Unit: helper functions for label resolution, parsing, and timestamp sorting.
- Integration: spawn temp git repos to validate each command, output, and prompt flow.
- E2E/manual: run `git-lock lock`, `git-lock list`, and `git-lock diff` in a real repo.

## Risks & gotchas
- Output parity is sensitive to whitespace, emoji, and prompt formatting.
- Timestamp parsing must remain cross-platform (macOS/Linux) for sorting and display.
- Tag creation and push behavior must avoid destructive actions without confirmation.

## Rollback plan
- Remove `crates/git-lock`, completion and wrapper files, and git-lock docs.
- Revert README/completion doc updates and keep the Zsh script as the active implementation.
