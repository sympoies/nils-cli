# Gemini Parity Cutover Checklist

This checklist is the release-day operator runbook for the Gemini parity lane
(`nils-gemini-core`, `nils-gemini-cli`, `nils-agent-provider-gemini`).

## Preflight

- [ ] Confirm clean working tree and expected branch/tag context.
- [ ] Verify required checks are green:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- [ ] Verify workspace coverage gate:
  - `mkdir -p target/coverage`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`
- [ ] Validate docs placement policy:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
- [ ] Validate Gemini publish order in `release/crates-io-publish-order.txt`:
  - `awk '/nils-gemini-core/{core=NR}/nils-gemini-cli/{cli=NR}/nils-agent-provider-gemini/{provider=NR} END{exit !(core>0 && cli>0 && provider>0 && core<cli && core<provider)}' release/crates-io-publish-order.txt`
- [ ] Run Gemini crates publish dry-run:
  - `scripts/publish-crates.sh --dry-run --crates "nils-gemini-core nils-gemini-cli nils-agent-provider-gemini"`

## Release

- [ ] Publish crates in dependency-safe order:
  1. `nils-gemini-core`
  2. `nils-gemini-cli`
  3. `nils-agent-provider-gemini`
- [ ] Verify `agentctl` sees stable Gemini provider metadata:
  - `cargo run -p nils-agentctl -- provider list --format json | rg '"name":"gemini"|"maturity":"stable"'`
- [ ] Verify healthcheck contract after publish:
  - `cargo run -p nils-agentctl -- provider healthcheck --provider gemini --format json`

## Post-release verification

- [ ] Confirm CLI smoke for text and JSON:
  - `cargo run -p nils-gemini-cli -- auth current`
  - `cargo run -p nils-gemini-cli -- auth current --json`
  - `cargo run -p nils-gemini-cli -- diag rate-limits --json`
- [ ] Confirm provider commands remain contract-compatible:
  - `cargo test -p nils-agent-provider-gemini --test adapter_contract`
  - `cargo test -p nils-agentctl --test provider_registry --test provider_commands`
- [ ] Confirm completion assets stay valid:
  - `zsh -n completions/zsh/_gemini-cli`
  - `bash -n completions/bash/gemini-cli`

## Rollback drill

- Trigger rollback when any release blocker appears:
  - provider contract regression
  - required checks/coverage regression
  - `agentctl provider healthcheck --provider gemini` fails post-release

- Rollback commands:
  - Revert provider maturity/behavior to stub baseline in `crates/agent-provider-gemini`.
  - Remove Gemini crates from immediate publish set while keeping branch artifacts for follow-up.
  - Re-run baseline validation:
    - `cargo test -p nils-agent-provider-gemini --test adapter_contract`
    - `cargo run -p nils-agentctl -- provider list --format json | rg '"gemini"|"stub"'`
    - `bash scripts/ci/docs-placement-audit.sh --strict`

- Expected rollback end-state:
  - Gemini provider appears as `stub` in provider list JSON.
  - No unstable Gemini release artifacts are promoted.
  - Workspace checks return to pre-cutover green state.
