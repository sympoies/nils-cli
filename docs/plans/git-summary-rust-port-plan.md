# Plan: Rust git-summary parity (CLI + completion + tests)

## Overview
This plan ports the existing Zsh `git-summary` implementation into a Rust CLI crate inside this
workspace, preserving output format, date-range behavior, and lockfile filtering from the current
Zsh implementation. It also snapshots the source Zsh script and completion into the repo for
repeatable parity reference, ports the zsh completion script, and adds a
Rust integration test suite (including edge cases) to ensure parity across presets and custom ranges.
The outcome is a `git-summary` binary with matching UX, a maintained completion file, and
repeatable tests that validate summary calculations and error handling.

## Scope
- In scope: Rust `git-summary` CLI implementation, output parity with the Zsh script, preset date
  range handling, lockfile filtering, zsh completion port, wrapper script, a source snapshot for
  parity reference, and a full test suite covering commands and edge cases.
- Out of scope: New subcommands, alternative output formats, or changes to git-summary UX beyond
  parity with the current Zsh script.

## Assumptions (if any)
1. The Rust CLI shells out to `git` for data collection (mirroring the Zsh script behavior).
2. Output text, table widths, and emojis match the current script output.
3. Date handling uses local timezone boundaries consistent with the script’s behavior.
4. Zsh completion file will live at `completions/zsh/_git-summary`.
5. Source snapshots for parity live under `docs/git-summary/source/`.
6. Tests can create temporary git repos and execute the new binary with stable output checks.

## Sprint 1: Parity spec + fixtures
**Goal**: Make current git-summary behavior explicit and capture fixtures for parity.
**Demo/Validation**:
- Command(s): `rg -n "git-summary" docs/git-summary/source/git-summary.zsh`, `rg -n "git-summary" docs/git-summary/source/_git-summary`
- Verify: Spec doc includes commands, help output, table format, and edge-case behavior.

### Task 1.1: Snapshot source script + completion + docs into repo
- **Location**:
  - `docs/git-summary/source/git-summary.zsh`
  - `docs/git-summary/source/_git-summary`
  - `docs/git-summary/source/git-summary.md`
- **Description**: Copy the current Zsh script, completion, and doc into repo-local snapshot files
  to make parity references reproducible without external paths.
- **Dependencies**:
  - none
- **Complexity**: 2
- **Acceptance criteria**:
  - Repo contains source snapshots for the script, completion, and doc.
- **Validation**:
  - `rg "git-summary" docs/git-summary/source/git-summary.zsh`
  - `rg "compdef" docs/git-summary/source/_git-summary`

### Task 1.2: Document current git-summary behavior and output contract
- **Location**:
  - `docs/git-summary/spec.md`
  - `docs/git-summary/source/git-summary.zsh`
  - `docs/git-summary/source/_git-summary`
  - `docs/git-summary/source/git-summary.md`
- **Description**: Read the Zsh implementation and docs to produce a concise spec covering commands,
  help text, date validation, output columns, sorting, and lockfile filtering.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Spec lists commands and custom range usage.
  - Spec captures date validation errors and missing-arg behavior.
  - Spec documents table columns, widths, and lockfile filtering rules.
- **Validation**:
  - `rg "Commands" docs/git-summary/spec.md`
  - `rg "lockfile" docs/git-summary/spec.md`

### Task 1.3: Capture fixture scenarios for tests
- **Location**:
  - `docs/git-summary/fixtures.md`
- **Description**: Define canonical test scenarios (custom range, presets, lockfile filtering,
  invalid date inputs, outside repo) and expected output markers.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Fixtures list covers presets + custom ranges and edge cases.
  - Each fixture includes setup steps and expected output markers.
- **Validation**:
  - `rg "##" docs/git-summary/fixtures.md`
  - `rg "edge" docs/git-summary/fixtures.md`

## Sprint 2: Rust crate scaffold + CLI surface
**Goal**: Add a new `git-summary` crate and CLI interface matching the script.
**Demo/Validation**:
- Command(s): `cargo metadata --no-deps | rg "git-summary"`, `cargo run -p git-summary -- --help`
- Verify: CLI help lists commands and custom range usage.

### Task 2.1: Create `git-summary` binary crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/git-summary/Cargo.toml`
  - `crates/git-summary/src/main.rs`
- **Description**: Add a new Rust binary crate named `git-summary` and register it in the workspace.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Workspace metadata lists `git-summary`.
  - `cargo run -p git-summary -- --help` succeeds.
- **Validation**:
  - `cargo metadata --no-deps | rg "git-summary"`
  - `cargo run -p git-summary -- --help`

### Task 2.2: Implement CLI parsing and help output
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Implement command parsing for presets (`today`, `yesterday`, `this-week`,
  `last-week`, `this-month`, `last-month`, `all`, `help`) and custom date ranges, matching the
  script’s help output and error messages.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Help output lists all commands and custom range usage.
  - Invalid usage prints the same error message as the script.
- **Validation**:
  - `cargo run -p git-summary -- --help | rg "this-week"`
  - `cargo run -p git-summary -- 2024-01-01 || true`

## Sprint 3: Core data collection + summary rendering
**Goal**: Port the summary computation and output formatting.
**Demo/Validation**:
- Command(s): `cargo run -p git-summary -- all`
- Verify: Table columns, widths, sorting, and totals match the script.

### Task 3.1: Implement git author collection
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Use `git log` with range args to collect unique authors (`%an` + email) and
  prepare per-author log calls.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Author list respects range filters and `--no-merges`.
  - No authors still prints the table header.
- **Validation**:
  - `cargo run -p git-summary -- all`

### Task 3.2: Compute per-author stats with lockfile filtering
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Parse `git log --numstat` output to compute added/deleted/net/commit counts,
  first/last commit dates, and exclude lockfiles from line counts.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Added/deleted counts ignore lockfiles.
  - Commit counts and first/last dates match the script behavior.
- **Validation**:
  - `cargo run -p git-summary -- all`

### Task 3.3: Render summary table and sorting
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Render the header, separator line, and per-author rows with fixed widths and
  sort by net contribution descending.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Output columns align with the script (widths and order).
  - Rows are sorted by net contribution descending.
- **Validation**:
  - `cargo run -p git-summary -- all`

## Sprint 4: Date validation + preset ranges
**Goal**: Match date validation behavior and preset range calculations.
**Demo/Validation**:
- Command(s): `cargo run -p git-summary -- today`, `cargo run -p git-summary -- this-week`
- Verify: Preset headers and date ranges render in local time.

### Task 4.1: Implement date format/value validation
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Validate `YYYY-MM-DD` format and ensure date values are valid, matching
  the script’s error messages.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Invalid format/value returns the same error strings.
  - Start date after end date returns the same range error.
- **Validation**:
  - `cargo test -p git-summary --test edge_cases invalid_date_format`
  - `cargo test -p git-summary --test edge_cases invalid_date_value`

### Task 4.2: Implement preset range calculations
- **Location**:
  - `crates/git-summary/src/main.rs`
- **Description**: Compute today/yesterday/this-week/last-week/this-month/last-month ranges in
  local time and pass range boundaries with timezone offsets to `git log`.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Preset commands print the same header lines as the script.
  - Boundaries match the expected Mon–Sun and month ranges.
- **Validation**:
  - `cargo test -p git-summary --test edge_cases preset_help_smoke`

## Sprint 5: Zsh completion + wrapper
**Goal**: Ship zsh completion and wrapper script aligned with current usage.
**Demo/Validation**:
- Command(s): `rg "git-summary" completions/zsh/_git-summary`
- Verify: Completion registers for `git-summary` and includes preset ranges.

### Task 5.1: Port zsh completion script
- **Location**:
  - `completions/zsh/_git-summary`
- **Description**: Port `~/.config/zsh/scripts/_completion/_git-summary` into this repo,
  preserving subcommands and date hints.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 3
- **Acceptance criteria**:
  - Completion file registers for `git-summary`.
  - Completion lists preset ranges and date hints.
- **Validation**:
  - `rg "git-summary command" completions/zsh/_git-summary`
  - `rg "this-week" completions/zsh/_git-summary`

### Task 5.2: Add wrapper script for git-summary
- **Location**:
  - `wrappers/git-summary`
- **Description**: Provide a wrapper script that runs the Rust binary or falls back to
  `cargo run -p git-summary`.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Wrapper invokes `git-summary` when installed.
  - Wrapper falls back to `cargo run` when binary missing.
- **Validation**:
  - `rg "cargo run -q -p git-summary" wrappers/git-summary`

## Sprint 6: Comprehensive test suite (including edge cases)
**Goal**: Cover summary calculations and edge-case behavior with repeatable tests.
**Demo/Validation**:
- Command(s): `cargo test -p git-summary`, `zsh -f tests/zsh/completion.test.zsh`
- Verify: Tests cover custom ranges, filtering, invalid input, and completion loading.

### Task 6.1: Add summary calculation integration tests
- **Location**:
  - `crates/git-summary/tests/summary_counts.rs`
  - `crates/git-summary/tests/common.rs`
- **Description**: Create temp repos, author commits with known dates, and assert table output
  (including lockfile filtering).
- **Dependencies**:
  - Task 3.3
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests assert added/deleted/net/commits and date columns.
  - Tests confirm lockfiles are excluded from counts.
- **Validation**:
  - `cargo test -p git-summary --test summary_counts`

### Task 6.2: Add edge case tests (invalid inputs, outside repo)
- **Location**:
  - `crates/git-summary/tests/edge_cases.rs`
- **Description**: Add tests for invalid date format/value, start > end, missing args, outside-repo,
  empty range output (no commits), numstat binary lines, and filenames with spaces.
- **Dependencies**:
  - Task 4.1
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Tests confirm error messages match the script.
  - Outside-repo case returns non-zero exit and warning text.
- **Validation**:
  - `cargo test -p git-summary --test edge_cases`

### Task 6.3: Extend zsh completion smoke test
- **Location**:
  - `tests/zsh/completion.test.zsh`
- **Description**: Add checks for `_git-summary` completion file load and preset list markers.
- **Dependencies**:
  - Task 5.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Zsh test validates `_git-summary` is defined and includes preset ranges.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

## Sprint 7: Documentation + validation pass
**Goal**: Document usage and validate repo-level workflows.
**Demo/Validation**:
- Command(s): `rg "git-summary" README.md`, `cargo test -p git-summary`
- Verify: Docs describe the new binary, wrappers, and completion setup.

### Task 7.1: Update README and completion docs
- **Location**:
  - `README.md`
  - `docs/completions-strategy.md`
- **Description**: Document the new `git-summary` binary and its completion/wrapper assets.
- **Dependencies**:
  - Task 5.2
- **Complexity**: 3
- **Acceptance criteria**:
  - README mentions `git-summary` usage.
  - Completion doc includes `_git-summary`.
- **Validation**:
  - `rg "git-summary" README.md`
  - `rg "_git-summary" docs/completions-strategy.md`

### Task 7.2: End-to-end validation
- **Location**:
  - `crates/git-summary`
  - `tests/zsh`
- **Description**: Run the full test suite and required validation commands.
- **Dependencies**:
  - Task 6.1
  - Task 6.2
  - Task 6.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `cargo test -p nils-common` passes.
  - `cargo test -p git-scope` passes.
  - `cargo test -p git-summary` passes.
  - `cargo test --workspace` passes.
  - `zsh -f tests/zsh/completion.test.zsh` passes.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo test -p git-scope`
  - `cargo test -p git-summary`
  - `cargo test --workspace`
  - `zsh -f tests/zsh/completion.test.zsh`

## Testing Strategy
- Unit: date parsing helpers and formatting utilities (if split into helpers).
- Integration: temp git repos for summary output and error cases.
- E2E/manual: run `git-summary all`, `git-summary this-month`, and a custom range in a real repo.

## Risks & gotchas
- Output parity is sensitive to table widths and whitespace alignment.
- Local timezone handling can affect boundary inclusion; tests should set explicit commit dates.
- Git log parsing must handle binary changes and lockfile exclusion consistently.

## Rollback plan
- Remove `crates/git-summary`, wrapper, and completion files.
- Revert docs changes and keep the Zsh script in `~/.config/zsh` as the active implementation.
