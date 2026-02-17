# Plan: macos-agent real-time feedback usability hardening for agent-in-the-loop development

## Overview
This plan hardens `macos-agent` for real-world agent development loops where commands are run repeatedly with immediate feedback, not as fixed idealized E2E scripts. Current coverage is strong for happy paths, but several high-friction areas remain: silent gating/skips in real-app tests, heavy reliance on fixed sleeps, panic-style failure reporting, and limited machine-parseable error context for fast automated remediation. The approach focuses on three outcomes: actionable feedback contracts, resilient real-app diagnostics, and ergonomic CLI workflows for rapid modify-test cycles.

## Related plans
- AX-first command surface and MVP delivery plan: `docs/plans/macos-agent-ax-subcommands-mvp-plan.md`

## Scope
- In scope:
  - Improve feedback quality for CLI failures and retries (structured diagnostics for fast triage).
  - Reduce false-green and flaky behavior in real-app test harness (`Arc`, `Spotify`, `Finder`).
  - Add operator-facing usability features for iterative testing (trace artifacts, scenario chaining, profile validation).
  - Expand deterministic and real-desktop test coverage around unstable/high-entropy workflows.
- Out of scope:
  - OCR/CV element detection or semantic UI understanding.
  - Non-macOS support.
  - Full workflow orchestration engine beyond bounded CLI scenario execution.

## Assumptions (if any)
1. `macos-agent` remains a CLI-first tool with stable stdout/stderr contracts for automation consumers.
2. Real-desktop tests remain opt-in and local-only; CI continues to rely on deterministic test mode.
3. Agent workflows primarily consume `--format json` and need parseable failure context, not only text stderr.
4. Compatibility-sensitive behavior can be preserved via opt-in flags when needed.

## Sprint 1: Feedback contract and diagnostics foundation
**Goal**: Make command outcomes immediately actionable for agents by improving parseable failures, retry visibility, and traceability.
**Demo/Validation**:
- Command(s):
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test retry -- --nocapture`
- Verify:
  - JSON users can parse failure payloads with operation, category, and remediation hints.
  - Action results expose actual attempt usage and trace locations.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` depends on both and integrates tracing end-to-end.

### Task 1.1: Add structured error payloads for JSON workflows
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/main.rs`
  - `crates/macos-agent/src/error.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/tests/contracts.rs`
- **Description**: Introduce structured JSON error output for automation-first flows (for example via `--error-format json`, with backward-compatible default text mode). Include stable fields for `category` (`usage`/`runtime`), `operation`, `message`, and `hints` so agents can auto-classify and remediate failures.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - JSON error mode emits machine-parseable payloads with deterministic keys.
  - Existing default text error behavior remains available and documented.
  - Error payloads include at least one actionable hint for common failures (permission, missing dependency, timeout).
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- error_commands_write_stderr_only_with_error_prefix --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test cli_smoke -- --nocapture`

### Task 1.2: Expose retry outcome telemetry in action results
- **Location**:
  - `crates/macos-agent/src/retry.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/src/commands/window_activate.rs`
  - `crates/macos-agent/src/commands/input_click.rs`
  - `crates/macos-agent/src/commands/input_type.rs`
  - `crates/macos-agent/src/commands/input_hotkey.rs`
  - `crates/macos-agent/tests/retry.rs`
- **Description**: Surface retry execution details (`attempts_used`, `retries_configured`, and terminal failure summary when applicable) in command results so agent loops can detect fragile steps and tune policies quickly.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Mutating action JSON includes actual attempts used.
  - Runtime timeout/non-zero failures preserve concise failure summaries.
  - Deterministic tests assert retry telemetry on both success-after-retry and fail-without-retry paths.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test retry -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test scenario_chain -- --nocapture`

### Task 1.3: Add per-action trace artifact emission for rapid debugging
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/model.rs`
  - `crates/macos-agent/tests/common.rs`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/README.md`
- **Description**: Add an optional trace sink (`--trace-dir` or equivalent) that writes per-action artifacts (request args, policy, elapsed time, status, stderr summary). Ensure artifact paths are deterministic in test mode and default safely to `AGENTS_HOME/out` when requested by users.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Trace mode produces one artifact per action with stable schema versioning.
  - Trace files are written for both success and failure paths.
  - README documents trace usage for iterative debugging loops.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test contracts -- --nocapture`
  - `rg -n "trace|AGENTS_HOME/out|schema_version" crates/macos-agent/README.md`

## Sprint 2: Real-app harness realism and failure visibility
**Goal**: Make real-app suites reflect non-ideal development reality by removing silent skips, reducing fixed sleeps, and improving per-step diagnostics.
**Demo/Validation**:
- Command(s):
  - `MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_apps -- real_e2e_contract_enforces_skip_vs_fail_policy --nocapture`
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- finder_navigation_and_state_checks_test --nocapture`
- Verify:
  - Invalid gating/configuration is explicit and actionable (not silent no-op success).
  - Step-level artifacts identify exactly where and why a flow failed.
  - Real-app commands are treated as manual local-macOS checks and kept out of CI defaults.

**Parallelization notes**:
- `Task 2.1` and `Task 2.2` can run in parallel.
- `Task 2.3` depends on `Task 2.2`.
- `Task 2.4` depends on `Task 2.2` and `Task 2.3`.

### Task 2.1: Replace silent gating with explicit skip/fail reporting
- **Location**:
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `crates/macos-agent/tests/e2e_real_macos.rs`
  - `crates/macos-agent/tests/real_apps/matrix.rs`
  - `crates/macos-agent/README.md`
- **Description**: Introduce a unified gating helper that records explicit skip reasons and rejects invalid app/profile selections with clear failures. Eliminate silent `return;` patterns that can produce false confidence in targeted runs.
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Unsupported `MACOS_AGENT_REAL_E2E_APPS` values produce actionable failure output.
  - Skipped scenarios emit machine-readable reason entries in artifact summaries.
  - README documents skip vs fail policy with concrete examples.
- **Validation**:
  - `cargo test -p macos-agent --test e2e_real_apps -- real_e2e_contract_enforces_skip_vs_fail_policy`
  - `cargo test -p macos-agent --test real_apps::matrix -- --nocapture`

### Task 2.2: Introduce a step ledger with stdout/stderr snapshots
- **Location**:
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/tests/real_apps/arc_navigation.rs`
  - `crates/macos-agent/tests/real_apps/arc_assertions.rs`
  - `crates/macos-agent/tests/real_apps/spotify_ui.rs`
  - `crates/macos-agent/tests/real_apps/spotify_state.rs`
  - `crates/macos-agent/tests/real_apps/finder.rs`
  - `crates/macos-agent/tests/real_apps/cross_app.rs`
- **Description**: Add a shared `run_step` wrapper that records command, arguments, attempts, elapsed time, and stdout/stderr excerpts into per-scenario `steps.jsonl`, plus failure checkpoint screenshot capture. This enables fast triage during iterative development.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - Every real-app scenario emits a step ledger artifact.
  - Failed runs include failing step id and last successful step id.
  - Artifact index links to the step ledger path.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- real_e2e_foundation_collects_artifacts --nocapture`
  - `cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_emits_artifact_index_with_required_fields`
  - `latest_dir="$(ls -td "${AGENTS_HOME:-$HOME/.agents}/out/macos-agent-e2e"/* 2>/dev/null | head -n 1)"; test -n "$latest_dir" && test -f "$latest_dir/steps.jsonl"`

### Task 2.3: Reduce fixed sleeps via condition-based waits
- **Location**:
  - `crates/macos-agent/tests/real_apps/arc_navigation.rs`
  - `crates/macos-agent/tests/real_apps/arc_assertions.rs`
  - `crates/macos-agent/tests/real_apps/spotify_ui.rs`
  - `crates/macos-agent/tests/real_apps/spotify_state.rs`
  - `crates/macos-agent/tests/real_apps/cross_app.rs`
- **Description**: Replace brittle fixed wait chains with condition-driven waits (`wait app-active`, `wait window-present`) and bounded fallback waits only where truly required. Keep thresholds explicit per step.
- **Dependencies**:
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Mutating scenario stages use condition waits before critical actions.
  - Any retained fixed waits are documented with rationale.
  - Scenario stability improves under moderate app/network latency variation.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- arc_youtube_multi_video_play_pause_and_comments_test --nocapture`
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=spotify MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- spotify_player_state_transitions_are_observable_test --nocapture`

### Task 2.4: Add retry safety boundaries for mutating scenarios
- **Location**:
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/tests/real_apps/arc_reliability.rs`
  - `crates/macos-agent/tests/real_apps/spotify_ui.rs`
  - `crates/macos-agent/tests/real_apps/cross_app.rs`
- **Description**: Introduce explicit retry categories (`idempotent` vs `mutating`) so retries do not replay unsafe multi-step chains. Fail fast with diagnostic context when a non-idempotent step fails.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Non-idempotent chain retries are disallowed by default.
  - Failures clearly state whether retry was attempted or blocked by policy.
  - Reliability module no longer retries panic-prone whole scenarios blindly.
- **Validation**:
  - `cargo test -p macos-agent --test e2e_real_apps -- arc_youtube_multi_video_play_pause_and_comments_test`
  - `cargo test -p macos-agent --test e2e_real_apps -- cross_app_arc_spotify_focus_and_state_recovery_test`

## Sprint 3: CLI ergonomics for immediate modify-test loops
**Goal**: Give agents first-class UX for running small action chains, validating profiles quickly, and checking readiness before mutating desktop state.
**Demo/Validation**:
- Command(s):
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo run -p macos-agent -- --format json scenario run --file crates/macos-agent/tests/fixtures/scenario-basic.json`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo run -p macos-agent -- profile validate --file crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
- Verify:
  - Agents can run compact scenario files with immediate step-by-step feedback.
  - Profile/config mistakes are caught before real-desktop mutation runs.

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel.
- `Task 3.3` depends on `Task 1.1`.
- `Task 3.4` depends on `Task 3.1`, `Task 3.2`, and `Task 3.3`.

### Task 3.1: Add `scenario run` command for chained operations
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/commands/mod.rs`
  - `crates/macos-agent/src/commands/scenario.rs`
  - `crates/macos-agent/tests/scenario_chain.rs`
  - `crates/macos-agent/tests/fixtures/scenario-basic.json`
- **Description**: Implement a bounded scenario runner that executes declarative step files (JSON) with per-step retry/timeout overrides, trace linkage, and summary output. This bridges single-command primitives with real iterative development loops.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 9
- **Acceptance criteria**:
  - Scenario runner executes multi-step chains with deterministic ordering.
  - Summary output reports succeeded/failed/skipped step counts and first failing step id.
  - Scenario mode works in deterministic test mode without real desktop mutation.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --test scenario_chain -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo run -p macos-agent -- --format json scenario run --file crates/macos-agent/tests/fixtures/scenario-basic.json`

### Task 3.2: Add profile validation and calibration bootstrap commands
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/run.rs`
  - `crates/macos-agent/src/commands/profile.rs`
  - `crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
  - `crates/macos-agent/README.md`
- **Description**: Add `profile validate` for schema/bounds checks and `profile init` (or equivalent) to generate a profile scaffold under `AGENTS_HOME/out`. This reduces coordinate drift errors before live runs.
- **Dependencies**:
  - none
- **Complexity**: 7
- **Acceptance criteria**:
  - Invalid/missing profile keys report exact key path and remediation message.
  - Validation checks include coordinate bounds sanity and required scenario points.
  - README documents profile bootstrap and update workflow.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo run -p macos-agent -- profile validate --file crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
  - `cargo test -p macos-agent --tests profile`

### Task 3.3: Extend preflight into live-loop readiness probes
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/src/preflight.rs`
  - `crates/macos-agent/tests/preflight.rs`
  - `crates/macos-agent/tests/e2e_real_macos.rs`
- **Description**: Extend preflight with optional actionable probes (`activate`, `input`, `screenshot`) that quickly estimate readiness for immediate testing loops and report per-probe risk level.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Preflight JSON includes probe rows with status and hint fields.
  - Probe failures map to clear mitigation instructions.
  - Deterministic tests cover probe success/failure permutations.
- **Validation**:
  - `cargo test -p macos-agent --test preflight -- --nocapture`
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo run -p macos-agent -- --format json preflight --strict`

### Task 3.4: Publish an immediate-feedback runbook and command recipes
- **Location**:
  - `crates/macos-agent/README.md`
  - `docs/plans/macos-agent-realtime-feedback-usability-hardening-plan.md`
- **Description**: Add a concise operator runbook for rapid loops (preflight, trace mode, scenario runs, profile validate, triage sequence). Include copy-paste command recipes for common failure classes.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
  - Task 3.3
- **Complexity**: 4
- **Acceptance criteria**:
  - README has a dedicated "Immediate Feedback Loop" section with at least 3 practical workflows.
  - Troubleshooting matrix maps common symptoms to next command.
  - Recipes are aligned with actual command surface introduced in this plan.
- **Validation**:
  - `rg -n "^## Immediate Feedback Loop$|Troubleshooting matrix|trace mode|scenario run|profile validate" crates/macos-agent/README.md`
  - `cargo run -p macos-agent -- --help | rg -n "scenario|profile"`

## Sprint 4: Coverage hardening and rollout safety
**Goal**: Ensure usability improvements are stable under fault conditions and safe to roll out without breaking existing consumers.
**Demo/Validation**:
- Command(s):
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --workspace -- --nocapture`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify:
  - New feedback/diagnostics features are covered by deterministic tests.
  - Existing command contracts stay stable (or explicitly versioned when changed).
  - Real-app repetition checks are manual-only and excluded from default CI gates.

**Parallelization notes**:
- `Task 4.1` and `Task 4.2` can run in parallel.
- `Task 4.3` depends on both and finalizes rollout controls.

### Task 4.1: Add deterministic fault-injection suite for live-loop failures
- **Location**:
  - `crates/macos-agent/tests/common.rs`
  - `crates/macos-agent/tests/retry.rs`
  - `crates/macos-agent/tests/window_activate.rs`
  - `crates/macos-agent/tests/input_click.rs`
  - `crates/macos-agent/tests/input_keyboard.rs`
  - `crates/macos-agent/tests/wait.rs`
- **Description**: Expand deterministic tests to cover high-friction faults seen in iterative loops: permission loss mid-run, stale target selectors, timeout under retries, malformed profile/config input, and trace directory write failures.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
  - Task 1.3
- **Complexity**: 7
- **Acceptance criteria**:
  - Each injected failure path has explicit assertion on actionable error content.
  - Fault tests remain CI-safe and deterministic.
  - New error payload contract is exercised for usage and runtime categories.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent --tests -- --nocapture`

### Task 4.2: Add real-app short-loop soak harness and pass-rate summary
- **Location**:
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `crates/macos-agent/tests/real_apps/matrix.rs`
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/README.md`
- **Description**: Add opt-in short-loop repetition mode for selected real-app scenarios with summarized pass/fail rate, dominant failure step ids, and artifact index paths to support practical flake triage.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
  - Task 2.4
- **Complexity**: 8
- **Acceptance criteria**:
  - Repetition mode supports app subset and iteration count inputs.
  - Output summary reports pass rate and top failing step ids.
  - Artifact index links each failed iteration to step ledger and screenshots.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_supports_app_subset_selection_real -- --nocapture --test-threads=1`
  - `echo "manual-only real-app validation; do not include in CI default test jobs"`

### Sprint 4 migration and rollout notes (Task 4.2 publish)
- Publish a README command decision matrix that explicitly maps `ax`/`input`/`wait` choices, fallback usage, and backend expectations.
- Set canonical invocation baseline for docs/examples: `--window-title-contains` and `input type --submit`.
- Keep compatibility aliases (`--window-name`, `input type --enter`) accepted during the current `0.x` rollout window; do not change default text output/error contracts in Sprint 4.
- Link decision matrix rows to troubleshooting and backend capability sections so fallback/backend incidents are diagnosable without changing command contracts.
- Verification command:
  - `rg -n "decision matrix|migration|alias|fallback|backend" crates/macos-agent/README.md`

### Task 4.3: Final compatibility checks and release guardrails
- **Location**:
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/tests/contracts.rs`
  - `crates/macos-agent/tests/cli_smoke.rs`
  - `crates/macos-agent/tests/e2e_real_macos.rs`
- **Description**: Finalize compatibility posture (default behavior vs opt-in enhancements), document migration notes, and enforce repository quality gates before rollout.
- **Dependencies**:
  - Task 4.1
  - Task 4.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Legacy-compatible output mode remains validated in contract tests.
  - New usability features are documented with explicit compatibility notes.
  - Required repository checks pass.
- **Validation**:
  - `cargo fmt --all -- --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test --workspace`
  - `zsh -f tests/zsh/completion.test.zsh`

## Testing Strategy
- Unit:
  - Validate error payload schema, retry telemetry mapping, and trace artifact serialization.
  - Validate profile parsing/validation and gating policy helpers.
- Integration:
  - Keep deterministic test mode as baseline for command contracts and fault injection.
  - Add scenario-runner integration tests with fixture-defined step files.
- E2E/manual:
  - Keep real-desktop suite opt-in only.
  - Add short-loop repetition mode with artifact-index + step-ledger outputs for triage.
- Regression guard:
  - Ensure existing text-mode contract remains covered where backward compatibility is required.

## Risks & gotchas
- Structured error payloads can break existing consumers if made default without migration guard.
- Additional diagnostics/trace writing can increase I/O overhead in tight loops.
- Real-desktop flake cannot be fully removed; only bounded and better diagnosed.
- Scenario runner scope can grow too broad; keep it bounded to avoid accidental workflow-engine creep.

## Rollback plan
- Keep new feedback formats and scenario runner behind explicit flags/subcommands until stable.
- Preserve legacy text error/output contracts as default during rollout window.
- If stability regresses, disable new real-app loop features while keeping artifact diagnostics from Sprint 2.
- Revert to pre-existing deterministic test suite as release gate while isolating new usability features behind opt-in paths.
