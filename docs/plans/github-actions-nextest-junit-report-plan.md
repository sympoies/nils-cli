# Plan: GitHub Actions cargo-nextest JUnit test report

## Overview
This plan adds machine-readable test reporting to CI by running Rust tests with `cargo-nextest`, generating a JUnit XML report, and publishing it as a GitHub Actions check + job summary. The goal is to make the workflow UI show which test cases ran (and which failed) without needing to download artifacts. The existing formatting/linting and zsh completion checks remain in CI.

## Scope
- In scope:
  - Run Rust tests in CI via `cargo nextest run` and generate JUnit XML via `.config/nextest.toml`.
  - Publish JUnit results into the GitHub Actions UI (check run + job summary) and upload the raw XML as an artifact.
  - Keep coverage parity for doctests by running `cargo test --workspace --doc` in CI (nextest currently doesn’t run doctests).
- Out of scope:
  - Changing the test suite itself (adding/removing/renaming tests).
  - Converting the zsh completion test to JUnit (it remains a log-only step unless explicitly added later).
  - Reworking the local required check entrypoint (`./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`) beyond documentation, unless Sprint 2 is executed.

## Assumptions (if any)
1. Adding a third-party GitHub Action for publishing JUnit results (e.g. `mikepenz/action-junit-report`) is acceptable for this repo.
2. “看到報告” means “visible in the GitHub Actions UI” (Checks tab / job summary), not only as a downloadable artifact.
3. CI should continue to run on both `pull_request` and `push` events as currently configured.

## Sprint 1: JUnit report visible in CI
**Goal**: A CI run publishes a JUnit report in the GitHub UI showing executed test cases and failures, and uploads the XML as an artifact.
**Demo/Validation**:
- Command(s): `cargo nextest run --profile ci --workspace`, `ls -la target/nextest/ci/junit.xml`
- Verify: `target/nextest/ci/junit.xml` exists locally after the run; in CI, a “JUnit Test Report” check is created and contains a detailed table of test cases.

### Task 1.1: Add nextest JUnit configuration
- **Location**:
  - `.config/nextest.toml`
- **Description**: Add a minimal `.config/nextest.toml` enabling JUnit output for the `ci` profile. Keep defaults for output storage (store failure output, do not store success output) to avoid huge XML files.
- **Dependencies**: none
- **Complexity**: 2
- **Acceptance criteria**:
  - Running `cargo nextest run --profile ci --workspace` creates `target/nextest/ci/junit.xml`.
  - The generated XML includes `testsuite` and `testcase` elements (not empty).
- **Validation**:
  - `cargo nextest run --profile ci --workspace`
  - `test -f target/nextest/ci/junit.xml`

### Task 1.2: Install cargo-nextest and run it in GitHub Actions
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Update the CI workflow to install `cargo-nextest` (via `taiki-e/install-action`) and run `cargo nextest run --profile ci --workspace` as the main Rust test runner. Add a separate doctest step (`cargo test --workspace --doc`) to preserve coverage parity with `cargo test --workspace`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - CI runs `cargo fmt --all -- --check` and `cargo clippy --all-targets --all-features -- -D warnings` as before.
  - CI runs `cargo nextest run --profile ci --workspace` and produces `target/nextest/ci/junit.xml` in the workspace.
  - CI still runs `zsh -f tests/zsh/completion.test.zsh`.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo nextest run --profile ci --workspace`
  - `cargo test --workspace --doc`

### Task 1.3: Publish JUnit results into the workflow UI
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Add a publish step using `mikepenz/action-junit-report@v6` that reads `target/nextest/ci/junit.xml` and creates a GitHub check run + job summary. Enable `detailed_summary` (and optionally `group_suite`) so the UI shows the individual test cases. Ensure the step runs even if the nextest step fails, but skip it when the XML doesn’t exist (e.g. formatting/lint failures before tests).
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - On PRs, the workflow run shows a “JUnit Test Report” check with a detailed summary table (test cases visible).
  - When nextest fails, the report step still runs and surfaces failing test cases.
  - When nextest never ran (no XML present), the report step is skipped (no extra noisy failures).
- **Validation**:
  - `git diff -- .github/workflows/ci.yml`

### Task 1.4: Upload the raw JUnit XML as an artifact
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Upload `target/nextest/ci/junit.xml` via `actions/upload-artifact` for debugging and offline inspection. Keep it conditional on the file existing and run it even on test failures.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 2
- **Acceptance criteria**:
  - A CI run exposes a downloadable artifact containing `junit.xml`.
  - The artifact upload step runs on both success and test failure (when the XML exists).
- **Validation**:
  - `git diff -- .github/workflows/ci.yml`

## Sprint 2: Polish and reduce maintenance burden (optional)
**Goal**: Make the workflow easier to maintain and document how to reproduce CI-style reporting locally.
**Demo/Validation**:
- Command(s): `rg -n "nextest|JUnit" DEVELOPMENT.md`, `cargo nextest run --profile ci --workspace`
- Verify: dev docs explain CI reporting and the doctest caveat; local reproduction works.

### Task 2.1: Document CI test reporting and the doctest caveat
- **Location**:
  - `DEVELOPMENT.md`
- **Description**: Update the development guide to include an optional “CI-style test reporting” section: how to install/run `cargo-nextest`, how to generate `target/nextest/ci/junit.xml`, and why doctests still run via `cargo test --workspace --doc`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - `DEVELOPMENT.md` documents the CI test runner (`cargo nextest run --profile ci --workspace`) and the JUnit output path.
  - `DEVELOPMENT.md` explicitly notes that doctests aren’t run by nextest and must be run separately.
- **Validation**:
  - `rg -n \"nextest\" DEVELOPMENT.md`
  - `rg -n \"doctest\" DEVELOPMENT.md`

### Task 2.2: Optional: Reuse the existing nils-cli checks script in CI
- **Location**:
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `.github/workflows/ci.yml`
- **Description**: If you want CI to keep using the single entrypoint script, extend `nils-cli-checks.sh` with an opt-in mode (e.g. env `NILS_CLI_TEST_RUNNER=nextest`) that runs nextest + doctests instead of `cargo test --workspace`. Then update CI to call the script in that mode while keeping the JUnit publish/upload steps.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 5
- **Acceptance criteria**:
  - Local default behavior remains unchanged: `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh` still runs `cargo test --workspace`.
  - CI uses nextest through the script and continues to generate/publish `target/nextest/ci/junit.xml`.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `NILS_CLI_TEST_RUNNER=nextest ./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: no new unit tests required (CI wiring and config changes only).
- Integration: validate nextest output locally by generating `target/nextest/ci/junit.xml` and ensuring it is well-formed XML.
- E2E/manual: open a PR and confirm GitHub Actions shows a “JUnit Test Report” check with a detailed test-case table and a downloadable artifact.

## Risks & gotchas
- `cargo-nextest` does not run doctests; keep `cargo test --workspace --doc` in CI if you rely on doctests.
- JUnit “detailed summary” can become large for big suites; if summaries become unwieldy, disable `detailed_summary` and rely on artifacts for the full list.
- For PRs from forks, repository security settings can restrict `GITHUB_TOKEN` permissions; the publish step requires `checks: write`.

## Rollback plan
- Revert `.github/workflows/ci.yml` to the previous `nils-cli-checks.sh`-only flow and remove the JUnit publish/upload steps.
- Remove `.config/nextest.toml` if nextest is no longer used in CI.
