# Plan: Rust git-scope parity (CLI + completion + tests)

## Overview
This plan ports the existing Zsh `git-scope` implementation into a Rust CLI crate inside this workspace,
keeping behavioral and output parity with `https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-scope.zsh`. It also ports the
Zsh completion script and recreates/extends the existing Zsh tests in Rust or zsh-driven integration
tests to ensure parity (including print modes, merge commit handling, and no-color behavior).
The outcome is a `git-scope` binary with matching UX, a maintained completion file, and a repeatable
test suite that covers the original script end-to-end.

## Scope
- In scope: Rust `git-scope` CLI implementation, output parity with existing Zsh script, zsh completion
  script port, wrapper/alias compatibility for `gs`/`gsc`/`gst`, and a full test suite covering all
  subcommands and edge cases (including print modes and merge parent selection).
- Out of scope: Porting unrelated zsh helpers, changing git-scope behavior/UX, or introducing new
  subcommands beyond the current feature set.

## Assumptions (if any)
1. The Rust CLI will shell out to `git` for data collection (not libgit2), mirroring the script logic.
2. Output text and structure (including emojis and labels) should be as close as possible to the
   existing `git-scope` script and docs.
3. Optional dependencies (`tree`, `file`) remain optional; absence should produce the same warnings
   as the Zsh script.
4. Zsh completion file will live at `completions/zsh/_git-scope` and remain compatible with `git-scope`
   and alias `gs`.
5. Tests can create temporary git repos and execute the new binary with `NO_COLOR=1` for stable output.

## Sprint 1: Baseline parity spec + fixture capture
**Goal**: Make the existing behavior explicit and capture fixtures for parity.
**Demo/Validation**:
- Command(s): `rg -n "git-scope" crates/git-scope/README.md`, `rg -n "compdef" completions/zsh/_git-scope`
- Verify: Spec doc includes subcommands, flags, output sections, and edge-case behavior.

### Task 1.1: Document current git-scope behavior and output contract
- **Location**:
  - `crates/git-scope/README.md`
  - `https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-scope.zsh`
  - `https://github.com/graysurf/zsh-kit/blob/main/docs/cli/git-scope.md`
- **Description**: Read the current Zsh implementation and documentation to produce a concise spec
  covering subcommands, flags, output sections, color behavior, and tree/print fallbacks.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Spec lists subcommands, flags (`-p`, `--print`, `--no-color`, `--parent/-P`), and output sections.
  - Spec captures merge commit parent selection behavior and warning messages.
  - Spec documents tree rendering and binary file output placeholders.
- **Validation**:
  - `rg "^##" crates/git-scope/README.md`
  - `rg "--parent" crates/git-scope/README.md`
  - `rg "tree" crates/git-scope/README.md`

### Task 1.2: Capture fixture scenarios for tests
- **Location**:
  - `crates/git-scope/README.md`
- **Description**: Define canonical test scenarios (staged/unstaged/both, untracked, tracked prefix,
  commit diff, merge commit parent selection, binary file, tree missing) and expected output sections
  to guide test creation.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Fixture list covers all subcommands and print modes.
  - Each fixture describes setup steps and expected output markers.
- **Validation**:
  - `rg "^##" crates/git-scope/README.md`
  - `rg "commit" crates/git-scope/README.md`

## Sprint 2: Rust crate scaffold + CLI surface
**Goal**: Add a new `git-scope` crate and CLI interface matching the current script.
**Demo/Validation**:
- Command(s): `cargo metadata --no-deps | rg "git-scope"`, `cargo run -p git-scope -- --help`
- Verify: CLI help lists subcommands and flags matching the spec.

### Task 2.1: Create `git-scope` binary crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/git-scope/Cargo.toml`
  - `crates/git-scope/src/main.rs`
- **Description**: Add a new Rust binary crate named `git-scope` and register it as a workspace member.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace metadata lists `git-scope`.
  - `cargo run -p git-scope -- --help` succeeds.
- **Validation**:
  - `cargo metadata --no-deps | rg "git-scope"`
  - `cargo run -p git-scope -- --help`

### Task 2.2: Implement CLI parsing with clap
- **Location**:
  - `crates/git-scope/src/main.rs`
- **Description**: Implement subcommands (`tracked`, `staged`, `unstaged`, `all`, `untracked`, `commit`)
  and flags (`-p/--print`, `--no-color`, `--parent/-P`) with help text aligned to the Zsh script.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - Help output lists all subcommands and options.
  - Invalid subcommand shows a clear error (matching existing UX expectations).
- **Validation**:
  - `cargo run -p git-scope -- --help | rg "tracked"`
  - `cargo run -p git-scope -- commit --help | rg "parent"`

## Sprint 3: Core data collection + renderers (worktree/index)
**Goal**: Port non-commit subcommands and file-content printing behavior.
**Demo/Validation**:
- Command(s): `cargo run -p git-scope -- staged -p`, `cargo run -p git-scope -- all -p`
- Verify: Output mirrors Zsh behavior for staged/unstaged/all and print sources.

### Task 3.1: Implement git status collection helpers
- **Location**:
  - `crates/git-scope/src/git.rs`
  - `crates/git-scope/src/main.rs`
- **Description**: Wrap `git diff --name-status`/`git ls-files` logic to match `_git_scope_collect`
  behavior, including tracked prefix filtering and `all` union output.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Tracked prefix filtering matches the Zsh script for directories, files, and raw prefixes.
  - `all` combines staged + unstaged uniquely and preserves status codes.
- **Validation**:
  - `cargo test -p git-scope --test tracked_prefix`

### Task 3.2: Render name-status output with colors and tree fallback
- **Location**:
  - `crates/git-scope/src/render.rs`
- **Description**: Implement output formatting for `📄 Changed files` and `📂 Directory tree`, including
  color mapping (A/M/D/U/-) and `tree --fromfile` fallback messaging.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Output headings and lines match the script (including emojis and labels).
  - `--no-color` and `NO_COLOR` disable ANSI sequences.
  - Missing `tree` prints the same warning text as the script.
- **Validation**:
  - `NO_COLOR=1 cargo run -p git-scope -- staged | rg "Changed files"`
  - `NO_COLOR=1 cargo run -p git-scope -- tracked | rg "Directory tree"`

### Task 3.3: Implement file content printing (worktree + index)
- **Location**:
  - `crates/git-scope/src/print.rs`
- **Description**: Port `print_file_content` and `print_file_content_index`, including binary detection,
  HEAD fallback, and output headers.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `staged -p` prints index content only.
  - `all -p` prints index + worktree for staged+unstaged files.
  - Binary files produce placeholder output.
- **Validation**:
  - `cargo test -p git-scope --test print_sources`

## Sprint 4: Commit inspection parity
**Goal**: Port `git-scope commit` behavior including merge parent selection.
**Demo/Validation**:
- Command(s): `cargo run -p git-scope -- commit HEAD`, `cargo run -p git-scope -- commit HEAD --parent 1`
- Verify: Metadata, commit message, stats, totals, and tree output match the spec.

### Task 4.1: Implement commit metadata and message rendering
- **Location**:
  - `crates/git-scope/src/commit.rs`
- **Description**: Render commit hash/author/date with optional color and print commit message body
  with indentation matching the script.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Output matches label/emoji format from the script.
  - `NO_COLOR=1` removes ANSI sequences.
- **Validation**:
  - `NO_COLOR=1 cargo run -p git-scope -- commit HEAD | rg "Commit Message"`

### Task 4.2: Implement commit file list with numstat totals and merge parent selection
- **Location**:
  - `crates/git-scope/src/commit.rs`
- **Description**: Port name-status/numstat parsing, totals calculation, merge parent selection rules,
  and warning messages when parent index is invalid.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Merge commits accept `--parent/-P` and fallback to parent #1 with warnings.
  - Output includes per-file stats and total line counts.
  - File list is stored for optional print output.
- **Validation**:
  - `cargo test -p git-scope --test commit_mode commit_merge_parent`

### Task 4.3: Add commit `-p/--print` file output
- **Location**:
  - `crates/git-scope/src/commit.rs`
  - `crates/git-scope/src/print.rs`
- **Description**: Reuse print helpers to output file contents for commit file list, matching the
  script’s `📦 Printing file contents` section.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `git-scope commit COMMIT -p` prints file content sections after the tree.
- **Validation**:
  - `cargo test -p git-scope --test commit_mode commit_print`

## Sprint 5: Zsh completion + wrapper compatibility
**Goal**: Ship zsh completion and wrapper aliases aligned with current usage.
**Demo/Validation**:
- Command(s): `rg "git-scope" completions/zsh/_git-scope`, `zsh -f ./scripts/tests/completion-smoke.zsh`
- Verify: Completion lists subcommands and supports commit hash completion.

### Task 5.1: Port zsh completion script
- **Location**:
  - `completions/zsh/_git-scope`
- **Description**: Port `https://github.com/graysurf/zsh-kit/blob/main/scripts/_completion/_git-scope` into this repo, preserving
  subcommands, options, commit hash completion, and alias support for `gs`.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Completion file registers for `git-scope` and `gs`.
  - Completion logic covers `commit` and tracked path prefixes.
- **Validation**:
  - `rg "compdef _git-scope git-scope gs" completions/zsh/_git-scope`
  - `rg "commit" completions/zsh/_git-scope`

### Task 5.2: Add wrapper scripts for aliases
- **Location**:
  - `wrappers/git-scope`
  - `wrappers/gs`
  - `wrappers/gsc`
  - `wrappers/gst`
- **Description**: Provide wrapper scripts that invoke the Rust binary and preserve alias behaviors
  from the Zsh script.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `gs` maps to `git-scope`.
  - `gsc` maps to `git-scope commit`.
  - `gst` maps to `git-scope tracked`.
- **Validation**:
  - `rg "git-scope" wrappers/gs`
  - `rg "git-scope commit" wrappers/gsc`
  - `rg "git-scope tracked" wrappers/gst`

## Sprint 6: Comprehensive test suite
**Goal**: Cover all behaviors from the Zsh script with repeatable tests.
**Demo/Validation**:
- Command(s): `cargo test -p git-scope`, `zsh -f tests/zsh/completion.test.zsh`
- Verify: Tests cover tracked prefix, print sources, commit merge handling, and no-color output.

### Task 6.1: Port existing Zsh tests into Rust integration tests
- **Location**:
  - `crates/git-scope/tests/print_sources.rs`
  - `crates/git-scope/tests/tracked_prefix.rs`
- **Description**: Recreate `git-scope-print-sources.test.zsh` and `git-scope-tracked-prefix.test.zsh`
  using Rust integration tests that spawn git repos and assert output (with `NO_COLOR=1`).
- **Dependencies**:
  - Task 3.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests mirror the logic and assertions of the Zsh originals.
  - Tests are deterministic (no color, stable temp paths).
- **Validation**:
  - `cargo test -p git-scope --test print_sources`
  - `cargo test -p git-scope --test tracked_prefix`

### Task 6.2: Add commit-mode parity tests (including merge parents)
- **Location**:
  - `crates/git-scope/tests/commit_mode.rs`
- **Description**: Create test repos with merge commits to validate `--parent/-P` behavior, totals,
  and warning messages for invalid indices.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover non-merge, merge parent selection, and invalid parent index fallbacks.
- **Validation**:
  - `cargo test -p git-scope --test commit_mode commit_merge_parent`

### Task 6.3: Add no-color + tree-missing tests
- **Location**:
  - `crates/git-scope/tests/rendering.rs`
- **Description**: Add tests for `NO_COLOR=1` output and for behavior when `tree` is missing
  (e.g., by overriding `PATH`).
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Output has no ANSI escape sequences when `NO_COLOR=1`.
  - Missing `tree` prints the warning line and does not fail.
- **Validation**:
  - `cargo test -p git-scope --test rendering no_color`
  - `cargo test -p git-scope --test rendering tree_missing`

### Task 6.4: Add completion smoke test (zsh)
- **Location**:
  - `tests/zsh/completion.test.zsh`
- **Description**: Add a minimal zsh test to source the completion file and assert that
  `git-scope` subcommands complete (basic smoke test, no full completion harness).
- **Dependencies**:
  - Task 5.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Test loads `_git-scope` without errors and lists expected subcommands.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

## Sprint 7: Documentation + validation pass
**Goal**: Document usage and validate repo-level workflows.
**Demo/Validation**:
- Command(s): `rg "git-scope" README.md`, `cargo test -p git-scope`
- Verify: Docs describe installation, wrappers, and completion setup.

### Task 7.1: Add README usage and install notes
- **Location**:
  - `README.md`
- **Description**: Document the new `git-scope` binary, wrapper scripts, and zsh completion setup
  in the repo docs.
- **Dependencies**:
  - Task 5.2
- **Complexity**: 3
- **Acceptance criteria**:
  - README includes `git-scope` usage and wrapper install notes.
  - README references the `_git-scope` completion file.
- **Validation**:
  - `rg "git-scope" README.md`
  - `rg "_git-scope" README.md`

### Task 7.2: End-to-end validation
- **Location**:
  - `crates/git-scope`
  - `tests/zsh`
- **Description**: Run the full test suite and basic manual checks for `git-scope` output.
- **Dependencies**:
  - Task 6.1
  - Task 6.2
  - Task 6.3
  - Task 6.4
- **Complexity**: 3
- **Acceptance criteria**:
  - `cargo test -p git-scope` passes.
  - Zsh completion smoke test passes.
- **Validation**:
  - `cargo test -p git-scope`
  - `zsh -f tests/zsh/completion.test.zsh`

## Testing Strategy
- Unit: helper functions (path filtering, parse/format) tested in Rust.
- Integration: spawn temp git repos to validate each subcommand and print behavior.
- E2E/manual: run `git-scope staged`, `git-scope all -p`, and `git-scope commit HEAD` in a real repo.

## Risks & gotchas
- Output parity is sensitive to whitespace, emoji, and color escape sequences.
- `tree --fromfile` behavior varies by installed `tree` version; tests should guard against missing
  or unsupported versions.
- Merge commit tests require deterministic parent ordering; ensure repo setup is stable.

## Rollback plan
- Remove `crates/git-scope`, wrapper scripts, and completion files.
- Revert docs changes and keep the existing Zsh implementation as the active reference:
  `https://github.com/graysurf/zsh-kit/blob/main/scripts/git/git-scope.zsh`.
