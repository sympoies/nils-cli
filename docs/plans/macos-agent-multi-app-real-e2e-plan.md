# Plan: macos-agent multi-app real desktop E2E coverage (Arc + Spotify + Finder)

## Overview
This plan expands `macos-agent` real-desktop coverage from basic readiness checks into app-driven E2E flows across Arc, Spotify, and Finder. The primary goal is to validate practical UI automation chains that include activation, click/typing/hotkey actions, wait stabilization, screenshot evidence, and app-state checks. The suite remains opt-in and local-macOS only to avoid CI flakiness, with explicit gating and artifact capture for triage.

## Scope
- In scope:
  - Add a dedicated real-desktop E2E suite for three apps: Arc, Spotify, and Finder.
  - Cover Arc YouTube flow: open YouTube, click multiple videos, play/pause, and navigate to comment area.
  - Cover Spotify flow: select a track through UI interactions, toggle play/pause, and assert app playback state.
  - Cover Finder flow: deterministic file-navigation workflow and window/app status verification.
  - Add shared harness utilities for app-availability checks, coordinate profiles, retries, and screenshot/log artifacts.
  - Document a repeatable local runbook and environment contract.
- Out of scope:
  - Running these real-desktop suites on CI.
  - OCR/CV-based semantic validation of rendered UI text.
  - Non-macOS support.
  - Automating account login bootstrap for Arc or Spotify.

## Assumptions (if any)
1. Test host is macOS with Accessibility, Automation, and Screen Recording permissions granted for the terminal host.
2. Arc and Spotify are installed; Spotify has an authenticated user session.
3. Network access to YouTube is available for Arc scenario execution.
4. Desktop resolution and scaling can vary, so coordinate profiles are required and selected explicitly.
5. Real-desktop tests are opt-in only and can be skipped when prerequisites are not met.

## Sprint 1: Real-E2E foundation and stability rails
**Goal**: Establish a reusable, diagnosable real-desktop E2E harness that app scenarios can build on.
**Demo/Validation**:
- Command(s):
  - `MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_apps -- real_e2e_foundation_reports_preflight_and_skip_reasons --nocapture`
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- real_e2e_foundation_collects_artifacts --nocapture`
- Verify:
  - Foundation test reports actionable skip reasons when prerequisites are missing.
  - Mutating run captures timestamped screenshots and step logs under `AGENTS_HOME/out/macos-agent-e2e/`.

**Parallelization notes**:
- `Task 1.1` and `Task 1.2` can run in parallel.
- `Task 1.3` depends on both foundation tasks.

### Task 1.1: Define real-app E2E contract and env matrix
- **Location**:
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/tests/e2e_real_apps.rs`
- **Description**: Define the canonical contract for real-app E2E runs: opt-in environment variables, skip policy, app prerequisites, profile selection, and artifact locations. Keep compatibility with existing `MACOS_AGENT_REAL_E2E` gating while extending it for multi-app selection.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - README documents `Arc`, `Spotify`, and `Finder` scenario gates and required permissions.
  - Test entrypoint documents each supported app scenario and when it should skip vs fail.
  - Environment contract includes profile and app selection variables with concrete examples.
- **Validation**:
  - `rg -n "MACOS_AGENT_REAL_E2E|MACOS_AGENT_REAL_E2E_MUTATING|MACOS_AGENT_REAL_E2E_PROFILE|MACOS_AGENT_REAL_E2E_APPS|Arc|Spotify|Finder" crates/macos-agent/README.md crates/macos-agent/tests/e2e_real_apps.rs`
  - `MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_apps -- real_e2e_contract_enforces_skip_vs_fail_policy --nocapture`

### Task 1.2: Build shared real-desktop harness helpers
- **Location**:
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
- **Description**: Implement shared helpers for real-desktop runs: app-installed probing, profile-based coordinate loading, resilient command execution wrappers, and standardized artifact capture (`json` output snapshots + screenshots + step transcript). Ensure all artifacts default to `AGENTS_HOME/out`.
- **Dependencies**:
  - none
- **Complexity**: 8
- **Acceptance criteria**:
  - Harness can load coordinate profile by name and fail with actionable error when missing.
  - Helpers provide a single `run_step` primitive with retry metadata and elapsed timing.
  - Every scenario can emit deterministic artifact directory names and paths.
- **Validation**:
  - `cargo test -p macos-agent --test e2e_real_apps -- real_common_profile_loader_and_artifact_paths_are_deterministic`
  - `MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_apps -- real_e2e_foundation_reports_preflight_and_skip_reasons --nocapture`

### Task 1.3: Add readiness smoke and failure-diagnostics baseline
- **Location**:
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `crates/macos-agent/tests/e2e_real_macos.rs`
- **Description**: Add baseline readiness checks that run before app flows: `preflight` JSON contract, app process visibility, and active-window stability probes. Standardize failure diagnostics so permission issues and focus drift are surfaced as actionable test errors.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Baseline checks distinguish between skip-worthy prerequisite gaps and hard failures.
  - Failure messages include the failing command, relevant stderr, and artifact path.
  - Existing `e2e_real_macos` checks remain valid and aligned with shared helpers.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_macos -- --nocapture`
  - `MACOS_AGENT_REAL_E2E=1 cargo test -p macos-agent --test e2e_real_apps -- real_e2e_foundation_reports_preflight_and_skip_reasons --nocapture`

## Sprint 2: Arc YouTube end-to-end scenario
**Goal**: Validate Arc browser automation on YouTube including multi-video interaction, playback toggling, and comment-section navigation.
**Demo/Validation**:
- Command(s):
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- arc_youtube_multi_video_play_pause_and_comments --nocapture`
- Verify:
  - Test activates Arc, navigates YouTube, opens multiple videos, toggles play/pause, and reaches comment area.
  - Step artifacts include screenshots at landing, per-video playback toggle, and comment-area checkpoint.

**Parallelization notes**:
- `Task 2.1` and `Task 2.2` can run in parallel after Sprint 1.
- `Task 2.3` depends on both to finalize reliability and diagnostics.
- Keep Arc implementation split by helper/assertion modules to avoid merge conflicts in one monolithic test file.

### Task 2.1: Implement Arc navigation and video-selection primitives
- **Location**:
  - `crates/macos-agent/tests/real_apps/arc_navigation.rs`
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
- **Description**: Add Arc-specific test helpers to activate Arc, normalize tab focus, open YouTube URLs, and click configurable video-tile coordinates. Keep selectors profile-driven so resolution-specific adjustments do not require test logic rewrites.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Helpers support at least three video-tile click targets from profile data.
  - Navigation sequence confirms Arc is frontmost before each click chain.
  - Failed clicks capture immediate screenshot and command trace.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- arc_youtube_opens_home_and_clicks_three_tiles --nocapture`

### Task 2.2: Implement Arc playback-toggle and comment-navigation assertions
- **Location**:
  - `crates/macos-agent/tests/real_apps/arc_assertions.rs`
  - `crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
- **Description**: Add assertions that each opened YouTube video can be toggled play/pause via keyboard focus path, then navigate to comment region through scroll/click choreography. Validate command-level success and app/window state before and after each checkpoint.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Scenario toggles playback state at least twice per selected video.
  - Scenario records a comment-region checkpoint for at least one video.
  - `wait app-active --app Arc` and `windows list --app Arc --window-name YouTube` checks pass at each stage.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- arc_youtube_play_pause_and_comment_checkpoint --nocapture`

### Task 2.3: Harden Arc suite against focus drift and transient latency
- **Location**:
  - `crates/macos-agent/tests/real_apps/arc_reliability.rs`
  - `crates/macos-agent/tests/real_apps/arc_navigation.rs`
  - `crates/macos-agent/tests/real_apps/arc_assertions.rs`
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/README.md`
- **Description**: Add bounded retries, explicit wait points, and richer diagnostics for Arc YouTube paths where network latency or animation can induce flakes. Document known failure signatures and remediation guidance.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Arc scenario retries only idempotent steps and fails fast on non-recoverable errors.
  - Failure output includes last successful step and artifact directory path.
  - README troubleshooting section includes Arc-specific focus and timing guidance.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- arc_youtube_multi_video_play_pause_and_comments --nocapture`
  - `rg -n "Arc|YouTube|focus drift|artifact" crates/macos-agent/README.md`

## Sprint 3: Spotify playback and app-state scenario
**Goal**: Validate Spotify UI interaction (track selection and play/pause) plus app state assertions suitable for agent workflows.
**Demo/Validation**:
- Command(s):
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=spotify MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- spotify_click_track_play_pause_and_state --nocapture`
- Verify:
  - Scenario opens Spotify, selects a track through UI steps, toggles play/pause, and asserts player state.
  - Artifacts include screenshot checkpoints and state snapshots (player state, track metadata).

**Parallelization notes**:
- `Task 3.1` and `Task 3.2` can run in parallel after Sprint 1.
- `Task 3.3` depends on both tasks.
- Keep Spotify scenario and status-probe logic in separate module files to minimize parallel edit contention.

### Task 3.1: Implement Spotify UI flow for search, select, and playback toggle
- **Location**:
  - `crates/macos-agent/tests/real_apps/spotify_ui.rs`
  - `crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
- **Description**: Build Spotify scenario steps that activate Spotify, focus search/navigation UI, click a deterministic track row from profile coordinates, and toggle play/pause through UI-focused inputs.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Scenario performs at least one UI-driven track selection and two play/pause toggles.
  - Every mutating step is preceded by app-active confirmation.
  - Scenario captures screenshot evidence before playback, during playback, and after pause.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=spotify MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- spotify_ui_selects_track_and_toggles_playback --nocapture`

### Task 3.2: Add Spotify app-state assertions and status capture
- **Location**:
  - `crates/macos-agent/tests/real_apps/spotify_state.rs`
  - `crates/macos-agent/tests/real_common.rs`
- **Description**: Add status probes to assert Spotify app state (`playing`/`paused`, current track metadata) after UI actions. Combine `macos-agent` checks (`wait app-active`, window visibility) with AppleScript status reads for stronger behavioral assertions.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - Test records at least these state fields: player state, track name, artist.
  - State assertions prove play/pause transitions map to expected status changes.
  - Probe failures include clear remediation when Spotify automation permission is blocked.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=spotify MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- spotify_player_state_transitions_are_observable --nocapture`

### Task 3.3: Add cross-app interruption check (Arc to Spotify and back)
- **Location**:
  - `crates/macos-agent/tests/real_apps/cross_app.rs`
  - `crates/macos-agent/tests/real_common.rs`
- **Description**: Add an interruption scenario that starts Spotify playback, switches to Arc for a short YouTube interaction, then returns to Spotify to verify status continuity and focus recovery. This validates realistic multi-app agent workflows.
- **Dependencies**:
  - Task 2.2
  - Task 2.3
  - Task 3.1
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Scenario verifies foreground app transitions using `wait app-active`.
  - Spotify status is captured before interruption and after returning focus.
  - Failure output pinpoints whether transition, Arc interaction, or Spotify resume failed.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc,spotify MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- cross_app_arc_spotify_focus_and_state_recovery --nocapture`

## Sprint 4: Finder scenario, matrix orchestration, and delivery docs
**Goal**: Complete the third app E2E flow with Finder and ship a maintainable multi-app execution matrix.
**Demo/Validation**:
- Command(s):
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- finder_navigation_and_state_checks --nocapture`
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc,spotify,finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- --nocapture`
- Verify:
  - Finder flow passes with deterministic setup/cleanup and state checks.
  - Full matrix run can execute requested app subsets and produce a unified artifact index.

**Parallelization notes**:
- `Task 4.2` starts after `Task 4.1` to reuse Finder scenario outputs in matrix baseline checks.
- `Task 4.3` depends on `Task 4.2`.
- `Task 4.4` depends on `Task 4.1` and `Task 4.3`.

### Task 4.1: Implement Finder deterministic workflow scenario
- **Location**:
  - `crates/macos-agent/tests/real_apps/finder.rs`
  - `crates/macos-agent/tests/fixtures/real_e2e_profile_default_1440p.json`
- **Description**: Add Finder real-desktop scenario that activates Finder, navigates to a known workspace folder, creates or opens a deterministic test directory, switches view mode, and verifies window/app status plus screenshot checkpoints.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Scenario has explicit setup and cleanup steps to avoid persistent desktop pollution.
  - Scenario asserts `wait app-active --app Finder` and `window-present` checkpoints.
  - Scenario captures at least two screenshots (post-navigation and post-action).
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- finder_navigation_and_state_checks --nocapture`

### Task 4.2: Add app-matrix core runner and artifact index generation
- **Location**:
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `crates/macos-agent/tests/real_apps/matrix.rs`
  - `crates/macos-agent/tests/real_common.rs`
  - `crates/macos-agent/README.md`
- **Description**: Add matrix execution helpers so one command can run any subset of `arc`, `spotify`, and `finder` scenarios. Emit a machine-readable artifact index summarizing per-scenario result, duration, and output paths.
- **Dependencies**:
  - Task 2.3
  - Task 4.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Matrix runner supports comma-separated app selection and preserves deterministic order.
  - Artifact index includes scenario id, status, elapsed time, and screenshot directory.
  - README provides example commands for single-app and full-matrix runs.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc,spotify,finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_supports_app_subset_selection --nocapture`
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc,spotify,finder MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_emits_artifact_index_with_required_fields --nocapture`
  - `rg -n "MACOS_AGENT_REAL_E2E_APPS|artifact index|arc,spotify,finder" crates/macos-agent/README.md`

### Task 4.3: Integrate cross-app scenario into matrix execution
- **Location**:
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `crates/macos-agent/tests/real_apps/cross_app.rs`
  - `crates/macos-agent/tests/real_apps/matrix.rs`
- **Description**: Wire cross-app Arc+Spotify interruption flow into matrix mode as an optional scenario so users can run base per-app coverage independently from higher-flake integration coverage.
- **Dependencies**:
  - Task 3.3
  - Task 4.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Matrix runner supports toggling extended cross-app scenario without forcing it in minimal runs.
  - Cross-app failures do not hide base per-app results in artifact index output.
  - Summary output clearly separates base scenarios from extended cross-app scenario.
- **Validation**:
  - `MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=arc,spotify MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_reports_base_and_extended_scenarios_separately --nocapture`

### Task 4.4: Finalize docs and mandatory quality gates
- **Location**:
  - `crates/macos-agent/README.md`
  - `crates/macos-agent/tests/e2e_real_apps.rs`
  - `docs/plans/macos-agent-multi-app-real-e2e-plan.md`
- **Description**: Finalize runbook documentation for prerequisites, known flaky points, and troubleshooting, then run required repo checks. Ensure the new E2E tests stay opt-in and do not regress existing deterministic suites.
- **Dependencies**:
  - Task 4.1
  - Task 4.3
- **Complexity**: 5
- **Acceptance criteria**:
  - README clearly separates deterministic CI-safe tests from real-desktop opt-in tests.
  - Existing `AGENTS_MACOS_AGENT_TEST_MODE=1` suite remains stable.
  - Required repo checks pass after test/doc updates.
- **Validation**:
  - `AGENTS_MACOS_AGENT_TEST_MODE=1 cargo test -p macos-agent`
  - `rg -n "Deterministic Test Mode|Opt-in Real macOS E2E Checks|MACOS_AGENT_REAL_E2E|MACOS_AGENT_REAL_E2E_MUTATING" crates/macos-agent/README.md`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`

## Testing Strategy
- Unit:
  - Keep scenario orchestration helpers, profile parsing, and artifact-index formatting unit-tested where possible.
- Integration:
  - Continue CI-safe integration tests under `AGENTS_MACOS_AGENT_TEST_MODE=1` for command contract and error semantics.
- E2E/manual:
  - Real-desktop suite runs opt-in on macOS with explicit app prerequisites and profile selection.
  - Required scenario minimum:
    - Arc: `arc_youtube_multi_video_play_pause_and_comments`
    - Spotify: `spotify_click_track_play_pause_and_state`
    - Finder: `finder_navigation_and_state_checks`
  - Recommended extended scenario:
    - `cross_app_arc_spotify_focus_and_state_recovery`
- Reliability/soak:
  - Run each base app scenario (`arc`, `spotify`, `finder`) for 5 consecutive repetitions.
  - Example:
    - `for app in arc spotify finder; do for i in 1 2 3 4 5; do MACOS_AGENT_REAL_E2E=1 MACOS_AGENT_REAL_E2E_MUTATING=1 MACOS_AGENT_REAL_E2E_APPS=$app MACOS_AGENT_REAL_E2E_PROFILE=default-1440p cargo test -p macos-agent --test e2e_real_apps -- matrix_runner_supports_app_subset_selection_real -- --nocapture --test-threads=1; done; done`
  - Target pass rate is at least 80 percent per scenario/profile combination before treating the flow as stable.
  - Any failed repetition must emit artifact index, failing step id, and last successful step id.

## Risks & gotchas
- UI coordinates can drift across display scaling, window layout, and app UI revisions.
- Arc and Spotify behavior can vary by account state, locale, and network availability.
- Real-desktop focus can be hijacked by notifications or other apps during runs.
- AppleScript state probes for Spotify can fail when Automation permissions are partially granted.
- Long-running app scenarios can accumulate flake if retries are unbounded.

## Rollback plan
- Keep all new scenarios behind `MACOS_AGENT_REAL_E2E=1` so default test runs remain unchanged.
- If instability is high, temporarily disable per-app scenarios by app-selection gate while keeping foundation diagnostics.
- Revert matrix orchestration to the previous `e2e_real_macos.rs` baseline and preserve only non-mutating readiness checks.
- Preserve deterministic coverage by relying on `AGENTS_MACOS_AGENT_TEST_MODE=1` tests until real-app stability is restored.
