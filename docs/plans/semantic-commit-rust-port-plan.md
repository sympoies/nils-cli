# Plan: Rust semantic-commit parity (CLI + completion + tests)

## Overview
This plan ports the existing Codex Zsh entrypoints under:
- `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/staged_context.sh`
- `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/commit_with_message.sh`

into a single Rust binary crate inside this workspace, named `semantic-commit`.
Behavioral parity (errors, warnings, exit codes, and validation rules) is the top priority.

## Scope
- In scope:
  - `semantic-commit` Rust crate + binary in `crates/semantic-commit`.
  - Subcommands: `staged-context`, `commit`, and `help`.
  - Commit message validation parity (header/body rules).
  - Codex command resolution parity for optional helpers (`git-commit-context-json`, `git-scope`).
  - Zsh completion file and wrapper script in-repo.
  - Comprehensive deterministic integration tests.
- Out of scope:
  - Changing the semantic commit rules (types/scope conventions) beyond what the scripts validate.
  - Running `git add` (autostage flows belong to a different tool/skill).

## Assumptions (if any)
1. `git` is available on `PATH` and can be invoked as a subprocess (mirrors the scripts).
2. The binary will often be installed under `$CODEX_HOME/commands/`, enabling CODEX_HOME inference.
3. Optional helper commands are resolved via Codex commands dir only (not general PATH), matching the
   source scripts.

## Sprint 1: Parity spec + fixtures
**Goal**: Make the current behavior explicit and define test fixtures.
**Demo/Validation**:
- `rg -n \"semantic-commit\" crates/semantic-commit/README.md`
- `rg -n \"^##\" crates/semantic-commit/README.md`

### Task 1.1: Document current behavior and output contract
- **Location**:
  - `crates/semantic-commit/README.md`
  - `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/staged_context.sh`
  - `https://github.com/graysurf/codex-kit/blob/main/skills/tools/devex/semantic-commit/scripts/commit_with_message.sh`
- **Description**: Capture CLI surface, preconditions, validation rules, error/warn text, and exit
  codes for both entrypoints.
- **Dependencies**: none
- **Complexity**: 3
- **Acceptance criteria**:
  - Spec includes exact error/warn strings, exit codes, and validation rules.
  - Spec documents Codex command resolution rules.
- **Validation**:
  - `rg -n \"invalid header format\" crates/semantic-commit/README.md`
  - `rg -n \"CODEX_COMMANDS_PATH\" crates/semantic-commit/README.md`

### Task 1.2: Define canonical fixture scenarios
- **Location**:
  - `crates/semantic-commit/README.md`
- **Description**: Enumerate deterministic test scenarios for both subcommands (success, fallbacks,
  and edge-case failures).
- **Dependencies**: Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - Fixtures cover staged-context fallback, outside-repo, no-staged, commit validation errors,
    and commit success output path.
- **Validation**:
  - `rg -n \"staged-context\" crates/semantic-commit/README.md`
  - `rg -n \"commit:\" crates/semantic-commit/README.md`

## Sprint 2: Crate scaffold + CLI surface
**Goal**: Add the new crate and implement the CLI entrypoints with custom help/usage.
**Demo/Validation**:
- `cargo run -p semantic-commit -- --help`
- `cargo run -p semantic-commit -- commit --help`

### Task 2.1: Create `semantic-commit` crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/semantic-commit/Cargo.toml`
  - `crates/semantic-commit/src/main.rs`
- **Description**: Add a new Rust binary crate named `semantic-commit` and register it as a
  workspace member.
- **Dependencies**: Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `cargo metadata` lists `semantic-commit`.
  - `cargo run -p semantic-commit -- --help` succeeds.
- **Validation**:
  - `cargo metadata --no-deps | rg \"semantic-commit\"`
  - `cargo run -p semantic-commit -- --help`

### Task 2.2: Implement top-level dispatch and subcommand usage
- **Location**:
  - `crates/semantic-commit/src/main.rs`
- **Description**: Implement `staged-context` and `commit` subcommands and help output, matching
  guardrails and error strings from the spec.
- **Dependencies**: Task 2.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Unknown args produce `error: unknown argument: ...` and show usage (when applicable).
  - `help`, `--help`, `-h`, and empty args print help and exit `0`.
- **Validation**:
  - `cargo run -p semantic-commit -- staged-context --help`
  - `cargo run -p semantic-commit -- commit --help`

## Sprint 3: Core functionality parity
**Goal**: Port behavior of both source scripts (including fallbacks).
**Demo/Validation**:
- Run against a temp git repo with staged changes and confirm output matches spec.

### Task 3.1: Implement staged-context behavior
- **Location**:
  - `crates/semantic-commit/src/staged_context.rs` (or equivalent module)
- **Description**: Implement precondition checks and context output preference:
  - prefer `git-commit-context-json --stdout --bundle` via Codex command resolution
  - fallback to `git diff --staged --no-color` with warning messages
- **Dependencies**: Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Exit codes and stderr warnings match the spec.
  - Output is pager-free (`GIT_PAGER=cat`, `PAGER=cat`).
- **Validation**:
  - `cargo test -p semantic-commit --test staged_context`

### Task 3.2: Implement commit behavior + message validation
- **Location**:
  - `crates/semantic-commit/src/commit.rs` (or equivalent module)
- **Description**: Implement commit message input (stdin/message/message-file), validate per spec,
  run `git commit -F`, then print a commit summary (git-scope preferred, git show fallback).
- **Dependencies**: Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - All validation errors match exact message text in the spec.
  - Git failures propagate exit codes and print the same wrapper error.
  - Summary output follows the same fallback warnings.
- **Validation**:
  - `cargo test -p semantic-commit --test commit`

## Sprint 4: Completion + wrapper + final gates
**Goal**: Add repo-standard wrappers and zsh completion, then pass all mandatory checks.

### Task 4.1: Add wrapper script
- **Location**:
  - `wrappers/semantic-commit`
- **Description**: Provide the same UX as other wrappers: if `semantic-commit` exists on PATH use it,
  otherwise `cargo run -q -p semantic-commit -- ...`.
- **Dependencies**: Task 2.1
- **Complexity**: 2
- **Acceptance criteria**:
  - `wrappers/semantic-commit --help` works in a fresh workspace checkout.
- **Validation**:
  - `./wrappers/semantic-commit --help`

### Task 4.2: Add zsh completion file and extend completion test
- **Location**:
  - `completions/zsh/_semantic-commit`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add a completion file mirroring the repo pattern and ensure it can be sourced in
  CI. Update the zsh completion test to include the new file.
- **Dependencies**: Task 2.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Completion script defines `_semantic-commit` function and includes subcommand list.
  - `zsh -f tests/zsh/completion.test.zsh` passes.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 4.3: Pre-delivery validation
- **Location**:
  - workspace root
- **Description**: Run the required formatting/lint/tests gate.
- **Dependencies**: Task 3.1, Task 3.2, Task 4.1, Task 4.2
- **Complexity**: 2
- **Acceptance criteria**:
  - All mandatory checks pass.
- **Validation**:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Use integration tests that create temporary git repositories and perform real staging/commits.
- Use deterministic inputs and assert on stable stderr/stdout markers (prefer NO_COLOR where needed).
- Do not rely on external `git-scope` or `git-commit-context-json`; test fallbacks by default.
- Add at least one test that stubs `CODEX_COMMANDS_PATH` to simulate presence/failure of an optional
  helper command.

## Risks & gotchas
- `git commit` hooks can interfere; tests should disable signing and avoid user-global config reliance.
- TTY detection differs between shells/CI; keep the stdin-vs-tty rule explicit and tested where
  feasible.
- Command resolution parity must not accidentally start searching general PATH, or behavior will drift
  from the scripts.

## Rollback plan
- Remove the crate from `Cargo.toml` workspace members.
- Delete `crates/semantic-commit/`, `completions/zsh/_semantic-commit`, and
  `wrappers/semantic-commit`, and revert `tests/zsh/completion.test.zsh` changes.
