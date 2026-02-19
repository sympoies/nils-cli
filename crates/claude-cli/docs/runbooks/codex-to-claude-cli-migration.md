# codex-cli to claude-cli migration

## Command mapping

| codex-cli surface | claude-cli / agentctl path | Notes |
|---|---|---|
| `codex-cli agent prompt ...` | `claude-cli agent prompt ...` | Executes through `claude-core` runtime. |
| `codex-cli agent advice ...` | `claude-cli agent advice ...` | Uses advice template expansion. |
| `codex-cli agent knowledge ...` | `claude-cli agent knowledge ...` | Uses knowledge template expansion. |
| `codex-cli auth current` (Claude intent) | `claude-cli auth-state show` | Claude auth state is env-driven. |
| `codex-cli diag rate-limits` | `claude-cli diag healthcheck` + `agentctl diag doctor --provider claude` | No codex rate-limit table equivalent. |
| `codex-cli provider|debug|workflow|automation ...` | `agentctl ...` | Provider-neutral orchestration stays in `agentctl`. |

## Unsupported codex-only surfaces

- `codex-cli agent commit`
- `codex-cli starship`
- Codex secret lifecycle commands (`auth save|remove|sync|use`)

## Migration checklist

1. Export `ANTHROPIC_API_KEY`.
2. Verify readiness:
   - `claude-cli diag healthcheck --format json`
   - `agentctl diag doctor --provider claude --format json`
3. Migrate prompt/advice/knowledge invocations to `claude-cli agent ...`.
4. Keep codex-only surfaces on `codex-cli` when required.
