# codex/gemini dual-cli rollout runbook

## Purpose
Define deterministic rollout, gating, fallback, and rollback steps for dual-CLI operation:
`codex-cli` + `gemini-cli` with `agentctl` as provider-neutral orchestration.

## Routing Contract

Canonical routing ownership is defined in:

- `docs/specs/codex-gemini-unified-architecture-v1.md`
- `crates/agent-provider-gemini/docs/specs/codex-cli-gemini-parity-matrix-v1.md`

This runbook only covers rollout execution, cutover, fallback, and rollback.

## Rollout Gating Checklist

1. Confirm architecture contract is present and current:
   - `docs/specs/codex-gemini-unified-architecture-v1.md`
2. Confirm parity matrix is present and current:
   - `crates/agent-provider-gemini/docs/specs/codex-cli-gemini-parity-matrix-v1.md`
3. Confirm operator docs consistently describe when to use `codex-cli`, `gemini-cli`, and `agentctl`.
4. Confirm wrapper guidance documents dual-CLI routing without implying Gemini must go through codex-only paths.
5. Confirm docs placement audit passes with strict mode.
6. For non-doc changes, run required workspace checks and coverage gate from `DEVELOPMENT.md`.

## Release cutover checklist

1. Keep dependency-safe publish ordering in `release/crates-io-publish-order.txt`:
   - `nils-gemini-core` must appear before `nils-agent-provider-gemini` when both are listed.
   - `nils-gemini-cli` must appear after runtime dependencies are listed.
2. Keep `nils-codex-cli` and `nils-agentctl` in the release set during dual-CLI rollout.
3. Verify command ownership hints match actual behavior:
   - `codex-cli` for Codex provider-specific commands.
   - `gemini-cli` for Gemini provider-specific commands.
   - `agentctl` for provider-neutral orchestration.
4. Confirm staged cutover keeps unsupported Gemini surfaces explicitly documented as unsupported.

## fallback

- If `gemini-cli` is not shipped or fails readiness checks, route Gemini execution through
  `agentctl workflow run --provider gemini`.
- If migrated commands are invoked via `codex-cli` and forwarding fails, run the equivalent
  `agentctl <command>` directly.
- Keep Codex-specific operations on `codex-cli`; do not redirect Codex auth/rate-limit UX to Gemini
  paths.
- Preserve deterministic error categories for unsupported Gemini surfaces; do not introduce silent
  command substitution.

## rollback

1. Remove `gemini-cli` from release cutover while keeping `agentctl` + `agent-provider-gemini`
   active for Gemini access.
2. Preserve `codex-cli` release flow unchanged.
3. Re-run required checks after rollback edits.
4. Restore rollout only after gating checks pass again.

## Validation Commands

```bash
rg -n "gemini-core|gemini-cli|agent-provider-gemini|agentctl" docs/specs/codex-gemini-unified-architecture-v1.md
rg -n "exact|semantic|unsupported" crates/agent-provider-gemini/docs/specs/codex-cli-gemini-parity-matrix-v1.md
rg -n "cutover|fallback|rollback" docs/runbooks/codex-gemini-dual-cli-rollout.md
bash scripts/ci/docs-placement-audit.sh --strict
```
