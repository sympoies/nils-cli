# codex/claude dual-cli rollout runbook

## Purpose
Define deterministic rollout, gating, fallback, and rollback steps for dual-CLI operation:
`codex-cli` + `claude-cli` with `agentctl` as provider-neutral orchestration.

## Routing Contract

| Need | Command surface | Notes |
| --- | --- | --- |
| Codex provider-specific workflows | `codex-cli` | Keep existing codex behavior and JSON contracts stable. |
| Claude provider-specific workflows | `claude-cli` | Use when `claude-cli` is included in the shipped release set. |
| Provider-neutral orchestration and diagnostics | `agentctl` | Canonical for `provider`, `workflow`, `debug`, and provider-neutral `diag`. |
| Migrated `codex-cli` wrapper commands (`provider`, `debug`, `workflow`, `automation`) | `agentctl` | `wrappers/codex-cli` forwards and preserves guidance on failure. |

## Rollout Gating Checklist

1. Confirm architecture contract is present and current:
   - `docs/specs/codex-claude-unified-architecture-v1.md`
2. Confirm operator docs consistently describe when to use `codex-cli`, `claude-cli`, and `agentctl`.
3. Confirm wrapper guidance documents dual-CLI routing without implying Claude must go through codex-only paths.
4. Confirm docs placement audit passes with strict mode.
5. For non-doc changes, run required workspace checks and coverage gate from `DEVELOPMENT.md`.

## Release Cutover Checklist

1. Keep dependency-safe publish ordering in `release/crates-io-publish-order.txt`:
   - `nils-claude-core` must appear before `nils-agent-provider-claude` when both are listed.
   - `nils-claude-cli` must appear after runtime dependencies are listed.
2. Keep `nils-codex-cli` and `nils-agentctl` in the release set during dual-CLI rollout.
3. Verify command ownership hints match actual behavior:
   - `codex-cli` for Codex provider-specific commands.
   - `claude-cli` for Claude provider-specific commands.
   - `agentctl` for provider-neutral orchestration.

## fallback

- If `claude-cli` is not shipped or fails readiness checks, route Claude execution through
  `agentctl workflow run --provider claude`.
- If migrated commands are invoked via `codex-cli` and forwarding fails, run the equivalent
  `agentctl <command>` directly.
- Keep Codex-specific operations on `codex-cli`; do not redirect Codex auth/rate-limit UX to Claude
  paths.

## rollback

1. Remove `claude-cli` from release cutover while keeping `agentctl` + `agent-provider-claude`
   active for Claude access.
2. Preserve `codex-cli` release flow unchanged.
3. Re-run required checks after rollback edits.
4. Restore rollout only after gating checks pass again.

## Validation Commands

```bash
rg -n "ownership|codex-core|claude-core|codex-cli|claude-cli|agentctl" docs/specs/codex-claude-unified-architecture-v1.md
rg -n "claude-cli|agentctl|provider-neutral orchestration" docs/runbooks/wrappers-mode-usage.md
rg -n "gating|fallback|rollback|agentctl|codex-cli|claude-cli" docs/runbooks/codex-claude-dual-cli-rollout.md
bash scripts/ci/docs-placement-audit.sh --strict
```
