# Plan: Peekaboo roadmap MVP execution

## Overview
The plan prioritizes fast, low-risk wins first: machine-readable permission status and recording metadata outputs. It then adds diff-aware capture and configurable wait policy to reduce flakiness and redundant artifacts, followed by optional advanced diagnostics. All work is scoped to existing `macos-agent` and `screen-record` crates with contract-first testing and rollback-safe toggles.

## Scope
- In scope:
  - Implement `screen-record --metadata-out` JSON output.
  - Define and expose unified permission status JSON (`screen_recording`, `accessibility`, `automation`, `ready`, `hints[]`).
  - Implement `--if-changed` capture flow with deterministic hash threshold behavior.
  - Add wait-policy configurability for AX-driven flows where needed for stability.
  - Add tests and docs for all new flags/contracts.
- Out of scope:
  - Full AI semantic element detection pipeline.
  - Full contact-sheet/motion-interval pipeline in the first delivery pass.
  - Cross-platform redesign beyond current macOS + Linux behavior contracts.

## Assumptions (if any)
1. Existing users expect backward compatibility; all new behavior will be opt-in via new flags or additive output fields.
2. JSON contracts are treated as public interfaces and require stable key naming.
3. Permission APIs needed for status checks are available in current macOS integration paths used by `macos-agent` and `screen-record`.
4. For `--if-changed`, deterministic hashing with configurable threshold is sufficient for MVP.

## Execution notes
- Critical path:
  - `Task 1.1` + `Task 1.2` -> `Task 1.3` -> `Task 2.1` + `Task 2.2` -> `Task 2.3`.
- Parallelizable blocks:
  - `Task 1.1` and `Task 1.2` can run in parallel, then converge at `Task 1.3`.
  - `Task 2.2` docs/tests can start once `Task 2.1` contract stabilizes.
  - Sprint 3 is optional and should start only after Sprint 2 quality gate passes.

## Sprint 1: Metadata and unified permission status (Phase 1)
**Goal**: Deliver machine-readable observability and permission readiness with minimal compatibility risk.
**Demo/Validation**:
- Command(s):
  - `cargo test -p screen-record --test cli_smoke`
  - `cargo test -p screen-record --test recording_test_mode`
  - `cargo test -p macos-agent --test preflight`
  - `cargo test -p macos-agent --test contracts`
- Verify:
  - `--metadata-out` writes complete JSON metadata.
  - Permission status JSON schema is consistent and parseable across the two tools.

### Task 1.1: Add `screen-record --metadata-out` contract and writer
- **Location**:
  - `crates/screen-record/src/cli.rs`
  - `crates/screen-record/src/types.rs`
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/main.rs`
- **Description**: Introduce `--metadata-out PATH` and emit JSON containing `target`, `duration_ms`, `audio_mode`, `format`, `output_path`, `output_bytes`, `started_at`, `ended_at`, and nullable `error`.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - New flag appears in `--help`.
  - Metadata file is written on success and failure paths with deterministic key names.
  - Existing output behavior remains unchanged when `--metadata-out` is not provided.
- **Validation**:
  - `cargo test -p screen-record --test cli_smoke`
  - `cargo test -p screen-record --test recording_test_mode`
  - `cargo test -p screen-record --test recording_test_mode -- metadata_out_failure_path`

### Task 1.2: Add shared permission status schema and adapter layer
- **Location**:
  - `crates/macos-agent/src/preflight.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/screen-record/src/macos/permissions.rs`
  - `crates/screen-record/src/types.rs`
- **Description**: Define a consistent JSON shape for permission status (`screen_recording`, `accessibility`, `automation`, `ready`, `hints[]`) and map each tool’s native checks to that shared schema.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - Both tools can emit the same logical status fields for macOS permission readiness.
  - `ready` reflects aggregate status consistently.
  - Hints are actionable and stable for automation consumers.
- **Validation**:
  - `cargo test -p macos-agent --test preflight`
  - `cargo test -p macos-agent --test contracts`
  - `cargo test -p screen-record --test cli_smoke`

### Task 1.3: Add docs and contract tests for metadata + permission status
- **Location**:
  - `crates/screen-record/README.md`
  - `crates/macos-agent/README.md`
  - `crates/screen-record/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Document new flags and JSON schema; add/extend tests to lock key names and expected behavior.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 5
- **Acceptance criteria**:
  - README examples are runnable and match CLI behavior.
  - Contract tests fail on schema/key drift.
- **Validation**:
  - `cargo test -p screen-record --test cli_smoke`
  - `cargo test -p macos-agent --test contracts`

## Sprint 2: Diff-aware capture and wait policy hardening (Phase 2)
**Goal**: Reduce unnecessary captures and improve AX workflow stability in flaky UI conditions.
**Demo/Validation**:
- Command(s):
  - `cargo test -p macos-agent --test observe_screenshot`
  - `cargo test -p macos-agent --test wait`
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p screen-record --test selection`
- Verify:
  - `--if-changed` avoids duplicate artifacts when screen content is unchanged.
  - Wait policy settings are configurable and honored in runtime behavior.

### Task 2.1: Implement `--if-changed` capture with baseline hash threshold
- **Location**:
  - `crates/macos-agent/src/commands/observe.rs`
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/macos/screenshot.rs`
  - `crates/screen-record/src/types.rs`
- **Description**: Add opt-in `--if-changed` mode that computes baseline/current hashes and emits `changed`, `baseline_hash`, `current_hash`, `threshold`, and nullable `captured_path`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Repeated unchanged captures skip artifact write when configured.
  - Hash and threshold values are included in JSON output for auditability.
  - Capture continues normally when `--if-changed` is disabled.
- **Validation**:
  - `cargo test -p macos-agent --test observe_screenshot`
  - `cargo test -p screen-record --test selection`
  - `cargo test -p macos-agent --test observe_screenshot -- if_changed_payload_contract`

### Task 2.2: Add configurable wait policy controls for AX interactions
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/wait.rs`
  - `crates/macos-agent/src/commands/ax_click.rs`
  - `crates/macos-agent/src/commands/ax_type.rs`
  - `crates/macos-agent/src/commands/wait.rs`
- **Description**: Introduce configurable wait-policy inputs (poll interval, timeout, required-state hooks) and ensure runtime uses one normalized policy structure.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - Wait policy options are visible and coherent in help text.
  - AX commands respect the configured policy and surface clear timeout errors.
  - Existing defaults preserve backward compatibility.
- **Validation**:
  - `cargo test -p macos-agent --test wait`
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test wait -- wait_policy_flags`

### Task 2.3: Expand docs/examples and regression tests for Phase 2 features
- **Location**:
  - `crates/macos-agent/README.md`
  - `crates/screen-record/README.md`
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/observe_screenshot.rs`
  - `crates/screen-record/tests/cli_smoke.rs`
- **Description**: Add examples for `--if-changed` and wait-policy usage, then lock expected behavior with regression tests.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Docs demonstrate end-to-end usage with expected outputs.
  - Regressions in flag behavior or payload shape are caught by tests.
- **Validation**:
  - `cargo test -p macos-agent --test cli_smoke`
  - `cargo test -p macos-agent --test observe_screenshot`
  - `cargo test -p screen-record --test cli_smoke`

## Sprint 3: Optional advanced diagnostics (Phase 3)
**Goal**: Add richer post-run diagnostics only if Sprint 1-2 quality gates are green and consumers need deeper debugging artifacts.
**Demo/Validation**:
- Command(s):
  - `cargo test -p screen-record --test recording_test_mode`
  - `cargo test -p screen-record --test writer`
- Verify:
  - Advanced diagnostics are opt-in and do not affect default output performance.

### Task 3.1: Define diagnostics artifact contract (contact sheet + motion intervals)
- **Location**:
  - `crates/screen-record/src/types.rs`
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/README.md`
- **Description**: Specify a versioned, opt-in diagnostics JSON contract and artifact naming conventions before implementation.
- **Dependencies**:
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Contract defines required keys, optional keys, and versioning strategy.
  - Artifact paths and lifecycle rules are documented.
- **Validation**:
  - `cargo test -p screen-record --test cli_smoke`
  - `cargo test -p screen-record --test cli_smoke -- diagnostics_contract_schema`

### Task 3.2: Implement contact-sheet and motion-interval generation (opt-in)
- **Location**:
  - `crates/screen-record/src/macos/writer.rs`
  - `crates/screen-record/src/run.rs`
  - `crates/screen-record/src/types.rs`
- **Description**: Add gated generation of contact-sheet images and motion-interval summaries, emitted only when diagnostics mode is enabled.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 9
- **Acceptance criteria**:
  - Default runs do not generate diagnostics artifacts.
  - Diagnostics runs produce valid metadata and readable artifact files.
  - Failure in diagnostics generation is surfaced clearly without corrupting primary output.
- **Validation**:
  - `cargo test -p screen-record --test recording_test_mode`
  - `cargo test -p screen-record --test writer`

## Testing Strategy
- Unit:
  - Add focused tests for metadata serialization, permission status aggregation, hash-threshold comparison, and wait-policy normalization.
- Integration:
  - Extend CLI smoke/contract tests for additive flags and JSON stability.
- E2E/manual:
  - Run representative macOS flows (permission denied, permission granted, unchanged/changed capture pairs, AX wait timeout scenarios).
  - Commands:
    - `cargo test -p macos-agent --test preflight`
    - `cargo test -p macos-agent --test observe_screenshot`
    - `cargo test -p screen-record --test recording_test_mode`

## Risks & gotchas
- macOS permission state is environment-sensitive and can make tests flaky without deterministic test-mode stubs.
- Hash-based change detection may produce false positives/negatives if image normalization is inconsistent.
- Contact-sheet/motion diagnostics can inflate artifact size and runtime if not strictly opt-in.
- Public JSON contracts can break downstream tooling if keys drift; contract tests are mandatory.

## Rollback plan
- Keep all new behaviors behind additive flags (`--metadata-out`, `--if-changed`, diagnostics opt-in flag) so rollback can be done by disabling flags in callers.
- If a released flag is unstable, retain parsing compatibility but no-op the feature and emit warning in stderr until fixed.
- Revert schema additions only with compatibility shim: preserve old keys, mark deprecated, and remove in a later major-version change.
- If wait-policy changes regress reliability, restore previous defaults and gate new policy fields behind explicit flag use.
