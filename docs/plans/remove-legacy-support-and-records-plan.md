# Plan: Remove Legacy Support and Legacy Records Safely

## Overview
This plan removes legacy runtime support paths and legacy historical records across the workspace, while preserving canonical command behavior and current JSON contracts.  
The approach is safety-first: inventory and baseline first, then remove runtime legacy branches, then delete legacy documentation/records, and finally run full required checks plus coverage gates.  
For risky removals (notably Gemini home-path fallback and image-processing legacy transform flow), the plan includes explicit migration and rollback steps to prevent data or workflow breakage.

## Scope
- In scope: remove legacy redirects, fallback paths, backward-compatible aliases, and schema fallbacks that exist only for legacy behavior.
- In scope: remove legacy-focused docs/spec sections and archived legacy records in repository documentation.
- In scope: update tests/contracts so they enforce canonical non-legacy behavior only.
- In scope: add automated guardrails to prevent legacy behavior from re-entering code or docs.
- Out of scope: adding new feature surfaces unrelated to legacy removal.
- Out of scope: changing provider model defaults, JSON schema ids, or non-legacy command groups.
- Out of scope: release tagging/publishing workflow execution.

## Assumptions (if any)
1. "Legacy content" includes runtime compatibility branches, backward-compatibility aliases, and documentation that preserves or describes legacy behavior.
2. Canonical command topology remains `agent`, `auth`, `diag`, `config`, `starship`, `completion`.
3. Removing legacy runtime fallback is allowed if one-time migration is provided before fallback removal.
4. For this request, image-processing legacy transform paths are considered legacy and must be removed.

## Sprint 1: Inventory and Safety Baseline
**Goal**: Produce a complete legacy manifest and lock canonical behavior before any removals.
**Demo/Validation**:
- Command(s): `rg -n --hidden --glob '!.git' -S '\blegacy\b|backward-compatible|compatibility messaging|PreferModernWhenPresentOrLegacyMissing|window-name|--enter|top-level send' .`
- Verify: all legacy items are classified with owner, removal action, dependency, and validation command.

### Task 1.1: Build a complete legacy removal manifest
- **Location**:
  - `docs/reports/legacy-removal-manifest.md`
  - `crates/codex-cli/src/main.rs`
  - `crates/gemini-cli/src/main.rs`
  - `crates/nils-common/src/provider_runtime/paths.rs`
  - `crates/gemini-cli/src/provider_profile.rs`
  - `crates/api-testing-core/src/websocket/schema.rs`
  - `crates/macos-agent/src/cli.rs`
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
  - `docs/specs/codex-gemini-cli-parity-contract-v1.md`
  - `docs/specs/codex-gemini-runtime-contract.md`
  - `docs/runbooks/wrappers-mode-usage.md`
- **Description**: Create a file-level manifest with each legacy item classified as `runtime-support`, `test-or-fixture`, or `documentation-record`, including planned removal task id and validation command.
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Every file containing legacy runtime behavior is listed exactly once.
  - Every listed item has an explicit owner crate/doc area and removal task id.
  - Manifest includes blocking risks for high-impact removals.
- **Validation**:
  - `test -f docs/reports/legacy-removal-manifest.md`
  - `rg -n '^\\| .* \\| (runtime-support|test-or-fixture|documentation-record) \\| Task [0-9]+\\.[0-9]+ \\|' docs/reports/legacy-removal-manifest.md`
  - `rg -n --hidden --glob '!.git' -S '\blegacy\b' .`

### Task 1.2: Freeze canonical non-legacy behavior baseline
- **Location**:
  - `crates/codex-cli/tests/main_entrypoint.rs`
  - `crates/codex-cli/tests/dispatch.rs`
  - `crates/codex-cli/tests/parity_oracle.rs`
  - `crates/gemini-cli/tests/main_entrypoint.rs`
  - `crates/gemini-cli/tests/dispatch.rs`
  - `crates/gemini-cli/tests/parity_oracle.rs`
  - `crates/macos-agent/src/cli.rs`
  - `crates/api-testing-core/src/websocket/schema.rs`
  - `crates/image-processing/tests/edge_cases.rs`
- **Description**: Execute and record baseline results for canonical command paths so post-removal regressions are detectable and attributable.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 5
- **Acceptance criteria**:
  - Baseline test commands for all affected crates are recorded in the manifest.
  - Baseline captures include exit semantics for canonical command paths.
  - No baseline item depends on legacy-only behavior.
- **Validation**:
  - `cargo test -p nils-codex-cli --test main_entrypoint --test dispatch --test parity_oracle`
  - `cargo test -p nils-gemini-cli --test main_entrypoint --test dispatch --test parity_oracle`
  - `cargo test -p nils-macos-agent`
  - `cargo test -p nils-api-testing-core`
  - `cargo test -p nils-image-processing`
  - `rg -n 'Task 1\\.2' docs/reports/legacy-removal-manifest.md`
  - `for token in nils-codex-cli nils-gemini-cli nils-macos-agent nils-api-testing-core nils-image-processing; do rg -n "$token" docs/reports/legacy-removal-manifest.md; done`

### Task 1.3: Prepare one-time migration workflow for Gemini legacy home data
- **Location**:
  - `scripts/migrations/migrate-gemini-home-paths.sh`
  - `crates/gemini-cli/docs/runbooks/gemini-path-migration.md`
  - `crates/nils-common/src/provider_runtime/paths.rs`
  - `crates/gemini-cli/src/provider_profile.rs`
- **Description**: Add a deterministic one-time migration script and runbook that moves legacy Gemini auth/secret paths to modern paths before runtime fallback removal.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 7
- **Acceptance criteria**:
  - Migration script is idempotent and preserves file content/permissions where applicable.
  - Runbook defines exact pre-check and post-check commands.
  - Migration path is validated on a temporary HOME layout.
- **Validation**:
  - `bash scripts/migrations/migrate-gemini-home-paths.sh --help`
  - `tmp="$(mktemp -d)" && mkdir -p "$tmp/home/.config/gemini_secrets" "$tmp/home/.agents" && printf '{}' > "$tmp/home/.agents/auth.json" && HOME="$tmp/home" bash scripts/migrations/migrate-gemini-home-paths.sh --yes`
  - `test -d "$tmp/home/.gemini/secrets" && test -f "$tmp/home/.gemini/oauth_creds.json"`

## Sprint 2: Remove Runtime Legacy Support Paths
**Goal**: Delete legacy runtime branches and aliases while preserving canonical behavior.
**Demo/Validation**:
- Command(s): `rg -n -S 'handle_legacy_redirect|PreferModernWhenPresentOrLegacyMissing|window-name|top-level send' crates`
- Verify: no runtime code still routes through legacy compatibility paths.

### Task 2.1: Remove codex/gemini legacy redirect handlers
- **Location**:
  - `crates/codex-cli/src/main.rs`
  - `crates/gemini-cli/src/main.rs`
  - `crates/codex-cli/tests/dispatch.rs`
  - `crates/gemini-cli/tests/dispatch.rs`
  - `crates/codex-cli/tests/parity_oracle.rs`
  - `crates/gemini-cli/tests/parity_oracle.rs`
- **Description**: Remove `handle_legacy_redirect` command interception and related redirect guidance assertions so unknown legacy commands flow through canonical clap error handling.
- **Dependencies**:
  - `Task 1.2`
- **Complexity**: 6
- **Acceptance criteria**:
  - `handle_legacy_redirect` no longer exists in codex/gemini binaries.
  - Dispatch tests are rewritten to assert canonical invalid-command behavior.
  - Parity tests still verify codex/gemini parity for canonical command topology.
- **Validation**:
  - `if rg -n 'handle_legacy_redirect|no longer supported|use `codex-cli|use `gemini-cli' crates/codex-cli/src/main.rs crates/gemini-cli/src/main.rs; then exit 1; fi`
  - `cargo test -p nils-codex-cli --test dispatch --test main_entrypoint --test parity_oracle`
  - `cargo test -p nils-gemini-cli --test dispatch --test main_entrypoint --test parity_oracle`

### Task 2.2: Remove Gemini legacy home-path fallback from shared runtime
- **Location**:
  - `crates/nils-common/src/provider_runtime/profile.rs`
  - `crates/nils-common/src/provider_runtime/paths.rs`
  - `crates/gemini-cli/src/provider_profile.rs`
  - `crates/nils-common/tests/provider_runtime_contract.rs`
  - `crates/gemini-cli/tests/runtime_paths_config_contract.rs`
- **Description**: Replace `PreferModernWhenPresentOrLegacyMissing` logic with modern-only path resolution and enforce migration-first behavior.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 8
- **Acceptance criteria**:
  - Shared runtime no longer defines legacy-path fallback enum/branches.
  - Gemini provider profile resolves only modern home paths.
  - Contract tests assert modern-only path behavior.
- **Validation**:
  - `if rg -n 'PreferModernWhenPresentOrLegacyMissing|SECRET_HOME_LEGACY|AUTH_HOME_LEGACY' crates/nils-common crates/gemini-cli; then exit 1; fi`
  - `cargo test -p nils-common --test provider_runtime_contract`
  - `cargo test -p nils-gemini-cli --test runtime_paths_config_contract`

### Task 2.3: Remove websocket legacy top-level `send` fallback
- **Location**:
  - `crates/api-testing-core/src/websocket/schema.rs`
  - `crates/api-testing-core/src/websocket/runner.rs`
  - `crates/api-websocket/docs/specs/websocket-request-schema-v1.md`
  - `crates/api-websocket/tests/integration.rs`
  - `crates/api-websocket/tests/json_contract.rs`
- **Description**: Require explicit `steps` in websocket request files and delete implicit receive-step construction from top-level `send`.
- **Dependencies**:
  - `Task 1.2`
- **Complexity**: 7
- **Acceptance criteria**:
  - Parser rejects requests that omit `steps`.
  - Schema docs no longer mention top-level legacy `send` compatibility.
  - Integration and contract tests are updated to explicit-step fixtures only.
- **Validation**:
  - `if rg -n 'top-level send|legacy `send`|receiveTimeoutSeconds|or top-level send' crates/api-testing-core/src/websocket/schema.rs crates/api-websocket/docs/specs/websocket-request-schema-v1.md; then exit 1; fi`
  - `cargo test -p nils-api-testing-core`
  - `cargo test -p nils-api-websocket`

### Task 2.4: Remove backward-compatible aliases from macos-agent CLI
- **Location**:
  - `crates/macos-agent/src/cli.rs`
  - `crates/macos-agent/README.md`
- **Description**: Remove alias acceptance for `--window-name` and `input type --enter`, keeping only canonical flags.
- **Dependencies**:
  - `Task 1.2`
- **Complexity**: 6
- **Acceptance criteria**:
  - CLI arg definitions no longer include legacy alias bindings.
  - Alias-focused tests are removed or replaced with canonical-only tests.
  - README documents canonical flags only.
- **Validation**:
  - `if rg -n 'window-name|--enter alias|Backward-compatible aliases are still accepted|legacy --window-name alias' crates/macos-agent/src/cli.rs crates/macos-agent/README.md; then exit 1; fi`
  - `cargo test -p nils-macos-agent`

### Task 2.5: Remove image-processing legacy transform execution path
- **Location**:
  - `crates/image-processing/src/main.rs`
  - `crates/image-processing/src/processing.rs`
  - `crates/image-processing/tests/edge_cases.rs`
  - `crates/image-processing/README.md`
  - `BINARY_DEPENDENCIES.md`
- **Description**: Remove ImageMagick legacy transform subcommands and retain only modern flow (`svg-validate` and `convert --from-svg`), including docs and dependency policy cleanup.
- **Dependencies**:
  - `Task 1.2`
- **Complexity**: 10
- **Acceptance criteria**:
  - Legacy transform command routes are deleted from CLI validation and processing layers.
  - README/BINARY_DEPENDENCIES no longer describe legacy transform paths.
  - Image-processing tests pass on modern-only command surface.
- **Validation**:
  - `if rg -n 'Legacy transform|legacy transform|auto-orient|resize|rotate|crop|pad|flip|flop|optimize' crates/image-processing/README.md BINARY_DEPENDENCIES.md; then exit 1; fi`
  - `cargo run -q -p nils-image-processing -- --help | rg -n 'svg-validate|convert'`
  - `cargo test -p nils-image-processing`

## Sprint 3: Remove Legacy Documentation and Historical Records
**Goal**: Eliminate legacy references from contracts/runbooks/readmes and delete archived legacy record sets.
**Demo/Validation**:
- Command(s): `rg -n --hidden --glob '!.git' -S '\blegacy\b|Backward-compatible|Legacy redirect parity|archive/v0' docs crates BINARY_DEPENDENCIES.md`
- Verify: no active docs or specs preserve legacy behavior or legacy historical records.

### Task 3.1: Rewrite workspace and crate contracts to canonical-only statements
- **Location**:
  - `docs/specs/codex-gemini-cli-parity-contract-v1.md`
  - `docs/specs/codex-gemini-runtime-contract.md`
  - `docs/runbooks/wrappers-mode-usage.md`
  - `docs/runbooks/new-cli-crate-development-standard.md`
  - `crates/codex-cli/README.md`
  - `crates/gemini-cli/README.md`
  - `crates/api-testing-core/README.md`
  - `crates/macos-agent/README.md`
  - `crates/image-processing/README.md`
  - `BINARY_DEPENDENCIES.md`
- **Description**: Remove legacy compatibility statements and replace them with canonical behavior contracts and migration references where required.
- **Dependencies**:
  - `Task 2.1`
  - `Task 2.2`
  - `Task 2.3`
  - `Task 2.4`
  - `Task 2.5`
- **Complexity**: 7
- **Acceptance criteria**:
  - Docs no longer state that legacy groups/aliases/fallbacks are supported.
  - Runtime and parity specs align with updated runtime behavior.
  - Dependency policy no longer lists removed legacy runtime dependencies.
- **Validation**:
  - `if rg -n --hidden --glob '!.git' -S '\\blegacy\\b|Backward-compatible aliases are still accepted|Legacy redirect parity|legacy top-level groups' docs crates/*/README.md BINARY_DEPENDENCIES.md; then exit 1; fi`
  - `bash scripts/ci/docs-placement-audit.sh --strict`

### Task 3.2: Remove archived legacy specs and stale legacy references
- **Location**:
  - `crates/memo-cli/docs/specs/archive/v0/memo-cli-command-contract-v0.md`
  - `crates/memo-cli/docs/specs/archive/v0/memo-cli-json-contract-v0.md`
  - `crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
  - `crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
  - `crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md`
  - `crates/memo-cli/README.md`
- **Description**: Delete archived v0 legacy specs and clean links/wording in active memo-cli docs to remove legacy historical record retention.
- **Dependencies**:
  - `Task 3.1`
- **Complexity**: 5
- **Acceptance criteria**:
  - `archive/v0` docs are removed.
  - No remaining links/reference text point to removed archive files.
  - Active memo-cli docs remain internally consistent after archive removal.
- **Validation**:
  - `test ! -d crates/memo-cli/docs/specs/archive/v0`
  - `if rg -n 'archive/v0|memo-cli-command-contract-v0|memo-cli-json-contract-v0' crates/memo-cli docs README.md; then exit 1; fi`
  - `cargo test -p nils-memo-cli`

### Task 3.3: Remove residual `legacy` wording from source and tests
- **Location**:
  - `crates/codex-cli/src/rate_limits/writeback.rs`
  - `crates/codex-cli/tests/parity_oracle.rs`
  - `crates/gemini-cli/tests/parity_oracle.rs`
  - `crates/memo-cli/src/output/text.rs`
  - `crates/api-testing-core/src/env_file.rs`
  - `crates/api-testing-core/src/graphql/mutation.rs`
  - `crates/api-testing-core/src/graphql/schema_file.rs`
- **Description**: Rename test fixture labels, comments, and sample text that use legacy wording, ensuring semantics remain unchanged for non-legacy behavior.
- **Dependencies**:
  - `Task 3.1`
- **Complexity**: 4
- **Acceptance criteria**:
  - No source/test files use legacy wording for runtime or docs semantics.
  - Behavior-equivalent tests continue to pass after wording cleanup.
- **Validation**:
  - `if rg -n --hidden --glob '!.git' --glob '*.rs' -S '\\blegacy\\b' crates; then exit 1; fi`
  - `cargo test -p nils-codex-cli --test parity_oracle`
  - `cargo test -p nils-gemini-cli --test parity_oracle`
  - `cargo test -p nils-api-testing-core`

## Sprint 4: Guardrails, Full Validation, and Rollback Drill
**Goal**: Prove the workspace is legacy-free without regressions and keep it that way.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` and coverage gate command.
- Verify: all required checks pass and automation prevents legacy reintroduction.

### Task 4.1: Add CI guardrails to prevent legacy reintroduction
- **Location**:
  - `scripts/ci/docs-hygiene-audit.sh`
  - `scripts/ci/docs-placement-audit.sh`
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `DEVELOPMENT.md`
- **Description**: Add deterministic checks that fail when legacy keywords or removed legacy command surfaces reappear in code/docs.
- **Dependencies**:
  - `Task 3.1`
  - `Task 3.2`
  - `Task 3.3`
- **Complexity**: 8
- **Acceptance criteria**:
  - CI script exits non-zero for reintroduced legacy patterns.
  - Required-checks entrypoint includes the new legacy guardrails.
  - Development docs show local run instructions for the guardrail checks.
- **Validation**:
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`

### Task 4.2: Run full required checks and workspace coverage gate
- **Location**:
  - `DEVELOPMENT.md`
  - `target/coverage/lcov.info`
- **Description**: Execute mandatory repository checks and coverage threshold to prove no regression after legacy removal.
- **Dependencies**:
  - `Task 2.1`
  - `Task 2.2`
  - `Task 2.3`
  - `Task 2.4`
  - `Task 2.5`
  - `Task 3.1`
  - `Task 3.2`
  - `Task 3.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Required lint/test/completion commands pass.
  - Coverage remains `>= 85.00%`.
  - No legacy keyword matches remain in tracked code/docs.
- **Validation**:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
  - `if rg -n --hidden --glob '!.git' -S '\\blegacy\\b|Backward-compatible aliases are still accepted|Legacy redirect parity' .; then exit 1; fi`

### Task 4.3: Execute rollback drill for high-risk removals
- **Location**:
  - `crates/gemini-cli/docs/runbooks/gemini-path-migration.md`
  - `docs/reports/legacy-removal-manifest.md`
  - `$AGENT_HOME/out/legacy-removal-rollback-drill.log`
- **Description**: Rehearse rollback for Gemini migration and image-processing command-surface reduction by restoring previous behavior from isolated commits and re-running targeted checks.
- **Dependencies**:
  - `Task 4.2`
- **Complexity**: 5
- **Acceptance criteria**:
  - Rollback steps are executable and time-bounded.
  - Targeted validation passes both in forward and rollback states.
  - Drill output is recorded for release readiness review.
- **Validation**:
  - `test -f crates/gemini-cli/docs/runbooks/gemini-path-migration.md`
  - `test -f docs/reports/legacy-removal-manifest.md`
  - `mkdir -p "$AGENT_HOME/out" && touch "$AGENT_HOME/out/legacy-removal-rollback-drill.log"`
  - `cargo test -p nils-gemini-cli --test runtime_paths_config_contract`
  - `cargo test -p nils-image-processing`

## Dependency and Parallel Execution Map
- Initial chain: `Task 1.1` -> `Task 1.2` and `Task 1.3`
- Runtime batch A (parallel after Sprint 1): `Task 2.1`, `Task 2.3`, `Task 2.4`
- Runtime batch B (after `Task 1.3`): `Task 2.2`
- Runtime batch C (parallel with A/B but highest risk): `Task 2.5`
- Docs batch (parallel after runtime convergence): `Task 3.1` and `Task 3.2`
- Cleanup batch: `Task 3.3` after `Task 3.1`
- Finalization: `Task 4.1` -> `Task 4.2` -> `Task 4.3`

## Testing Strategy
- Unit:
  - `cargo test -p nils-codex-cli --test dispatch --test parity_oracle`
  - `cargo test -p nils-gemini-cli --test dispatch --test parity_oracle --test runtime_paths_config_contract`
  - `cargo test -p nils-api-testing-core`
  - `cargo test -p nils-macos-agent`
  - `cargo test -p nils-image-processing`
- Integration:
  - `cargo test -p nils-api-websocket`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- E2E/manual:
  - `cargo run -q -p nils-codex-cli -- provider`
  - `cargo run -q -p nils-gemini-cli -- provider`
  - `cargo run -q -p nils-image-processing -- --help`
  - `rg -n --hidden --glob '!.git' -S '\\blegacy\\b' .`
  - `zsh -f tests/zsh/completion.test.zsh`
  - `bash -n completions/bash/codex-cli`
  - `zsh -n completions/zsh/_codex-cli`

## Risks & gotchas
- Removing Gemini legacy path fallback can break users who still store credentials only in legacy locations if migration is skipped.
- Removing image-processing legacy transform paths can break automation scripts that still call removed subcommands.
- Removing websocket top-level `send` fallback can break older fixture files and suite manifests.
- Deleting archived memo docs can break external links if stale references remain.
- A strict "no legacy token" rule can create false positives if not scoped to meaningful contexts.

## Rollback plan
- Execute changes in isolated task-group commits so each high-risk area can be reverted independently.
- Keep migration/runbook changes in a separate commit from runtime fallback removal to allow partial rollback without losing migration guidance.
- If Gemini auth path regressions occur, immediately revert `Task 2.2` commit(s), re-run runtime path contract tests, and re-ship with migration gating only.
- If image-processing regressions occur, revert `Task 2.5` commit(s), preserve docs updates, and split command removals into smaller follow-up slices.
- If websocket schema changes regress suites, restore previous parser behavior from git history and keep fixture/doc changes staged for controlled re-application.
- After any rollback, rerun required checks and coverage gate before reattempting removal.
