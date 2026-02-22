# Legacy Removal Manifest (Sprint 1 Task 1.1)

## Scope and discovery

- Source plan: `docs/plans/remove-legacy-support-and-records-plan.md`.
- Discovery baseline command:
  - `rg -n --hidden --glob '!.git' -S '\blegacy\b|backward-compatible|compatibility messaging|PreferModernWhenPresentOrLegacyMissing|window-name|--enter|top-level send' .`
- Additional focused discovery commands were run per legacy surface (redirect handlers, Gemini path fallback, websocket top-level `send`, macOS aliases, image-processing legacy transforms).
- Inclusion rule used for this manifest: every Sprint 1 Task 1.1 location plus directly related runtime/test/doc files discovered by ripgrep for the same removal surfaces.

## File-level legacy manifest

| File | Classification | Planned removal task id | Owner area | Legacy item | Planned removal action | Dependency | Validation command |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `crates/codex-cli/src/main.rs` | runtime-support | Task 2.1 | `crates/codex-cli` | `handle_legacy_redirect` interception + compatibility guidance text | Delete legacy redirect handler and route legacy groups through canonical clap errors | Task 1.2 | if rg -n 'handle_legacy_redirect' crates/codex-cli/src/main.rs; then exit 1; fi |
| `crates/gemini-cli/src/main.rs` | runtime-support | Task 2.1 | `crates/gemini-cli` | `handle_legacy_redirect` interception + compatibility guidance text | Delete legacy redirect handler and route legacy groups through canonical clap errors | Task 1.2 | if rg -n 'handle_legacy_redirect' crates/gemini-cli/src/main.rs; then exit 1; fi |
| `crates/codex-cli/tests/dispatch.rs` | test-or-fixture | Task 2.1 | `crates/codex-cli` | Assertion for legacy redirect guidance (`codex-cli help`) | Replace with canonical invalid-command behavior assertions | Task 1.2 | if rg -n 'codex-cli help' crates/codex-cli/tests/dispatch.rs; then exit 1; fi |
| `crates/gemini-cli/tests/dispatch.rs` | test-or-fixture | Task 2.1 | `crates/gemini-cli` | Assertion for legacy redirect guidance (`gemini-cli help`) | Replace with canonical invalid-command behavior assertions | Task 1.2 | if rg -n 'gemini-cli help' crates/gemini-cli/tests/dispatch.rs; then exit 1; fi |
| `crates/codex-cli/tests/main_entrypoint.rs` | test-or-fixture | Task 2.1 | `crates/codex-cli` | Assertion that legacy commands print `no longer supported` | Replace with canonical clap usage-error assertions | Task 1.2 | if rg -n 'no longer supported' crates/codex-cli/tests/main_entrypoint.rs; then exit 1; fi |
| `crates/gemini-cli/tests/main_entrypoint.rs` | test-or-fixture | Task 2.1 | `crates/gemini-cli` | Assertion that legacy commands print `no longer supported` | Replace with canonical clap usage-error assertions | Task 1.2 | if rg -n 'no longer supported' crates/gemini-cli/tests/main_entrypoint.rs; then exit 1; fi |
| `crates/codex-cli/tests/parity_oracle.rs` | test-or-fixture | Task 2.1 | `crates/codex-cli` | Legacy command parity assertion text (`legacy command mismatch`) | Keep parity coverage but drop legacy-group expectation | Task 1.2 | if rg -n 'legacy command mismatch' crates/codex-cli/tests/parity_oracle.rs; then exit 1; fi |
| `crates/gemini-cli/tests/parity_oracle.rs` | test-or-fixture | Task 2.1 | `crates/gemini-cli` | Legacy command parity assertion text (`legacy command mismatch`) | Keep parity coverage but drop legacy-group expectation | Task 1.2 | if rg -n 'legacy command mismatch' crates/gemini-cli/tests/parity_oracle.rs; then exit 1; fi |
| `crates/nils-common/src/provider_runtime/profile.rs` | runtime-support | Task 2.2 | `crates/nils-common/provider_runtime` | `HomePathSelection::PreferModernWhenPresentOrLegacyMissing` enum variant | Remove fallback variant and retain modern-only home-path selection | Task 1.3 | if rg -n 'PreferModernWhenPresentOrLegacyMissing' crates/nils-common/src/provider_runtime/profile.rs; then exit 1; fi |
| `crates/nils-common/src/provider_runtime/paths.rs` | runtime-support | Task 2.2 | `crates/nils-common/provider_runtime` | Runtime branch resolving legacy home paths when modern path is missing | Remove legacy fallback branch from path resolution | Task 1.3 | if rg -n 'PreferModernWhenPresentOrLegacyMissing' crates/nils-common/src/provider_runtime/paths.rs; then exit 1; fi |
| `crates/gemini-cli/src/provider_profile.rs` | runtime-support | Task 2.2 | `crates/gemini-cli` | `SECRET_HOME_LEGACY` / `AUTH_HOME_LEGACY` constants + fallback profile config | Resolve Gemini runtime paths to modern-only locations | Task 1.3 | if rg -n 'SECRET_HOME_LEGACY' crates/gemini-cli/src/provider_profile.rs; then exit 1; fi |
| `crates/nils-common/tests/provider_runtime_contract.rs` | test-or-fixture | Task 2.2 | `crates/nils-common` | Contract fixtures asserting `.config/gemini_secrets` and `.agents/auth.json` fallback | Replace with migration-first, modern-only path contract tests | Task 1.3 | if rg -n 'GEMINI_SECRET_HOME_LEGACY' crates/nils-common/tests/provider_runtime_contract.rs; then exit 1; fi |
| `crates/api-testing-core/src/websocket/schema.rs` | runtime-support | Task 2.3 | `crates/api-testing-core/websocket` | Schema accepts missing `steps` via top-level `send` fallback | Require explicit `steps`; remove top-level `send` compatibility branch and test | Task 1.2 | if rg -n 'or top-level send' crates/api-testing-core/src/websocket/schema.rs; then exit 1; fi |
| `crates/macos-agent/src/cli.rs` | runtime-support | Task 2.4 | `crates/macos-agent` | `visible_alias = "window-name"` and `input type --enter` alias acceptance tests in file | Remove backward-compatible alias bindings | Task 1.2 | if rg -n 'visible_alias = \"window-name\"' crates/macos-agent/src/cli.rs; then exit 1; fi |
| `crates/macos-agent/tests/cli_smoke.rs` | test-or-fixture | Task 2.4 | `crates/macos-agent` | Smoke tests using `--window-name` and `--enter` legacy aliases | Replace with canonical-flag-only smoke tests | Task 1.2 | if rg -n '\"--window-name\"' crates/macos-agent/tests/cli_smoke.rs; then exit 1; fi |
| `crates/macos-agent/tests/wait.rs` | test-or-fixture | Task 2.4 | `crates/macos-agent` | Wait command test using `--window-name` legacy alias | Replace with canonical `--window` flag coverage | Task 1.2 | if rg -n '\"--window-name\"' crates/macos-agent/tests/wait.rs; then exit 1; fi |
| `crates/image-processing/src/cli.rs` | runtime-support | Task 2.5 | `crates/image-processing` | Legacy transform operation surface (`auto-orient`, `resize`, `rotate`, `crop`, `pad`, `flip`, `flop`, `optimize`) | Remove legacy transform subcommands from CLI operation map | Task 1.2 | if rg -n 'Operation::AutoOrient' crates/image-processing/src/cli.rs; then exit 1; fi |
| `crates/image-processing/src/main.rs` | runtime-support | Task 2.5 | `crates/image-processing` | Legacy transform validation/dispatch branches | Remove legacy transform validation paths and keep modern-only flow | Task 1.2 | if rg -n 'Operation::Optimize' crates/image-processing/src/main.rs; then exit 1; fi |
| `crates/image-processing/src/processing.rs` | runtime-support | Task 2.5 | `crates/image-processing` | Legacy ImageMagick execution path (`auto-orient/resize/rotate/crop/pad/flip/flop/optimize`) | Delete legacy operation execution branches and dependencies | Task 1.2 | if rg -n 'Operation::Optimize' crates/image-processing/src/processing.rs; then exit 1; fi |
| `crates/image-processing/tests/core_flows.rs` | test-or-fixture | Task 2.5 | `crates/image-processing` | Tests for legacy optimize pipelines (`cjpeg`, `cwebp`) | Replace/remove tests tied to deleted legacy optimize path | Task 1.2 | if rg -n 'optimize_uses_cjpeg_pipeline_when_available' crates/image-processing/tests/core_flows.rs; then exit 1; fi |
| `crates/image-processing/tests/dry_run_paths.rs` | test-or-fixture | Task 2.5 | `crates/image-processing` | Dry-run tests for legacy transform/optimize command assembly | Replace with modern-only dry-run coverage | Task 1.2 | if rg -n 'optimize_jpg_dry_run_falls_back_to_magick' crates/image-processing/tests/dry_run_paths.rs; then exit 1; fi |
| `crates/image-processing/tests/edge_cases.rs` | test-or-fixture | Task 2.5 | `crates/image-processing` | Edge-case tests anchored to legacy transform subcommands | Replace with modern-only edge-case contract coverage | Task 1.2 | if rg -n 'rotate_requires_degrees_is_usage_error' crates/image-processing/tests/edge_cases.rs; then exit 1; fi |
| `docs/specs/codex-gemini-cli-parity-contract-v1.md` | documentation-record | Task 3.1 | `docs/specs` | `Legacy redirect parity` section and unsupported legacy group contract text | Rewrite parity contract to canonical-only command surface | Tasks 2.1-2.5 | if rg -n 'Legacy redirect parity' docs/specs/codex-gemini-cli-parity-contract-v1.md; then exit 1; fi |
| `docs/specs/codex-gemini-runtime-contract.md` | documentation-record | Task 3.1 | `docs/specs` | Gemini home-path fallback text preserving legacy credential path behavior | Rewrite runtime contract to modern-only path behavior + migration references | Tasks 2.1-2.5 | if rg -n 'gemini_secrets' docs/specs/codex-gemini-runtime-contract.md; then exit 1; fi |
| `docs/runbooks/wrappers-mode-usage.md` | documentation-record | Task 3.1 | `docs/runbooks` | Compatibility messaging guidance for legacy top-level groups | Remove legacy-group guidance from wrapper runbook | Tasks 2.1-2.5 | if rg -n 'legacy top-level groups' docs/runbooks/wrappers-mode-usage.md; then exit 1; fi |
| `crates/api-websocket/docs/specs/websocket-request-schema-v1.md` | documentation-record | Task 2.3 | `crates/api-websocket/docs/specs` | Schema docs describing legacy top-level `send` compatibility | Update spec to require `steps` only | Task 1.2 | if rg -n 'missing-steps.ws.json' crates/api-websocket/docs/specs/websocket-request-schema-v1.md; then exit 1; fi |
| `crates/macos-agent/README.md` | documentation-record | Task 2.4 | `crates/macos-agent` | Statement that backward-compatible aliases are still accepted | Document canonical flags only | Task 1.2 | if rg -n 'Backward-compatible aliases are still accepted' crates/macos-agent/README.md; then exit 1; fi |
| `crates/image-processing/README.md` | documentation-record | Task 2.5 | `crates/image-processing` | Explicit legacy transform path and command matrix | Remove legacy transform documentation; keep modern-only guidance | Task 1.2 | if rg -n 'Legacy transform path' crates/image-processing/README.md; then exit 1; fi |
| `BINARY_DEPENDENCIES.md` | documentation-record | Task 2.5 | `workspace root docs` | Legacy transform dependency policy for ImageMagick toolchain | Remove legacy transform dependency section and retain modern dependency policy | Task 1.2 | if rg -n 'legacy transform subcommands' BINARY_DEPENDENCIES.md; then exit 1; fi |

## Sprint 1 Task 1.1 location coverage

| Sprint 1 location | Coverage status |
| --- | --- |
| `docs/reports/legacy-removal-manifest.md` | Covered: this artifact is the Task 1.1 manifest output |
| `crates/codex-cli/src/main.rs` | Covered |
| `crates/gemini-cli/src/main.rs` | Covered |
| `crates/nils-common/src/provider_runtime/paths.rs` | Covered |
| `crates/gemini-cli/src/provider_profile.rs` | Covered |
| `crates/api-testing-core/src/websocket/schema.rs` | Covered |
| `crates/macos-agent/src/cli.rs` | Covered |
| `crates/image-processing/src/main.rs` | Covered |
| `crates/image-processing/src/processing.rs` | Covered |
| `docs/specs/codex-gemini-cli-parity-contract-v1.md` | Covered |
| `docs/specs/codex-gemini-runtime-contract.md` | Covered |
| `docs/runbooks/wrappers-mode-usage.md` | Covered |

## Blocking risks for high-impact removals

| Risk | Affected files | Blocking condition | Mitigation / unblock validation |
| --- | --- | --- | --- |
| Gemini credential path fallback removal can hide existing auth/secrets if migration is skipped | `crates/nils-common/src/provider_runtime/paths.rs`, `crates/gemini-cli/src/provider_profile.rs`, `crates/nils-common/tests/provider_runtime_contract.rs`, `docs/specs/codex-gemini-runtime-contract.md` | Any environment still stores only legacy paths (`$HOME/.config/gemini_secrets`, `$HOME/.agents/auth.json`) before Task 2.2 lands | Complete Task 1.3 migration workflow first; validate by running migration script dry-run path checks and confirming modern targets exist |
| Image-processing legacy transform removal can break existing automation scripts and CI that call removed subcommands | `crates/image-processing/src/cli.rs`, `crates/image-processing/src/main.rs`, `crates/image-processing/src/processing.rs`, `crates/image-processing/tests/*`, `crates/image-processing/README.md`, `BINARY_DEPENDENCIES.md` | Any script, docs, or tests still invoke `auto-orient|resize|rotate|crop|pad|flip|flop|optimize` at cutover time | Migrate callers to `svg-validate` + `convert --from-svg`; validate via `cargo run -q -p nils-image-processing -- --help | rg -n 'svg-validate|convert'` and targeted test updates |
| Redirect-handler removal can break wrapper/tooling flows that parse current compatibility messaging strings | `crates/codex-cli/src/main.rs`, `crates/gemini-cli/src/main.rs`, `docs/runbooks/wrappers-mode-usage.md`, `docs/specs/codex-gemini-cli-parity-contract-v1.md` | External automation depends on exact `no longer supported` / `use <cmd>` output text | Freeze canonical error baseline in Task 1.2 and update wrapper docs + callers before Task 2.1 / Task 3.1 removals |
| Websocket top-level `send` fallback removal can break request fixtures that omit explicit `steps` | `crates/api-testing-core/src/websocket/schema.rs`, `crates/api-websocket/docs/specs/websocket-request-schema-v1.md` | Existing fixtures still rely on implicit receive-step generation from top-level `send` | Update fixtures to explicit `steps`; validate in Task 2.3 with schema + integration test runs |

## Sprint 1 Task 1.2 baseline validation results (canonical non-legacy behavior)

- Execution date: `2026-02-22`
- Purpose: lock a reproducible pre-removal baseline for canonical command/test surfaces and exit semantics before Sprint 2 runtime legacy removals.

| Validation command | Exit code | Notes (canonical behavior / non-legacy reliance) |
| --- | --- | --- |
| `cargo test -p nils-codex-cli --test main_entrypoint --test dispatch --test parity_oracle` | `0` | Baseline for `nils-codex-cli` command topology/dispatch/entrypoint semantics; pass does not require legacy filesystem fallbacks or legacy-only runtime paths. |
| `cargo test -p nils-gemini-cli --test main_entrypoint --test dispatch --test parity_oracle` | `0` | Baseline for `nils-gemini-cli` canonical command behavior parity with codex surfaces; pass does not require legacy-only credential path fallback to succeed. |
| `cargo test -p nils-macos-agent` | `0` | Baseline for `nils-macos-agent` canonical CLI/runtime behavior and exit semantics before alias removals; pass is reproducible without legacy-only external state. |
| `cargo test -p nils-api-testing-core` | `0` | Baseline for `nils-api-testing-core` schema/runtime contracts prior to removing websocket top-level `send` compatibility branch; pass does not depend on legacy-only transport paths. |
| `cargo test -p nils-image-processing` | `0` | Baseline for `nils-image-processing` canonical processing and CLI exit semantics before legacy transform route removal; pass does not require legacy-only environment migration state. |

### Task 1.2 required validation tokens

- `nils-codex-cli`
- `nils-gemini-cli`
- `nils-macos-agent`
- `nils-api-testing-core`
- `nils-image-processing`
