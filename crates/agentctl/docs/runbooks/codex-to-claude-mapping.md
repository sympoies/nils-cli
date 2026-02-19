# codex-cli to Claude mapping

## Purpose
Map codex-oriented intents to the canonical Claude surface in dual-CLI operation.

## When to use which surface

| Intent class | Preferred surface | Fallback surface |
| --- | --- | --- |
| Codex provider-specific workflows | `codex-cli` | none |
| Claude provider-specific workflows | `claude-cli` | `agentctl workflow run --provider claude` |
| Provider-neutral orchestration/diagnostics | `agentctl` | none |

## Intent mapping

| codex-cli intent | Claude-first path | Provider-neutral path (`agentctl`) | Notes |
| --- | --- | --- | --- |
| `codex-cli agent prompt ...` | `claude-cli agent prompt ...` | `agentctl workflow run --provider claude ...` | Prompt intent remains explicit in both paths. |
| `codex-cli agent advice ...` | `claude-cli agent advice ...` | `agentctl workflow run --provider claude ...` | Advice intent remains provider-specific UX. |
| `codex-cli agent knowledge ...` | `claude-cli agent knowledge ...` | `agentctl workflow run --provider claude ...` | Knowledge intent remains provider-specific UX. |
| `codex-cli auth *` | `claude-cli auth-state ...` | `agentctl provider healthcheck --provider claude` | Claude auth is env/config driven, not Codex secret-file lifecycle. |
| `codex-cli diag rate-limits` | `claude-cli diag ...` | `agentctl diag doctor --provider claude` | Codex rate-limit table has no direct Claude equivalent. |
| `codex-cli config show/set` | `claude-cli config ...` | workflow/provider env validation in `agentctl` | Use provider-specific config commands for user workflows. |
| `codex-cli starship` | unsupported | unsupported | Codex-specific UX surface. |

## Unsupported codex-only surfaces for Claude

- `agent commit`
- `starship`
- Codex secret store lifecycle commands (`auth save/remove/sync/use`)

## Recommended migration steps

1. Set required Claude environment variables.
2. Verify provider readiness:
   - `cargo run -p nils-agentctl -- provider healthcheck --provider claude --format json`
   - `cargo run -p nils-agentctl -- diag doctor --provider claude --format json`
3. Use `claude-cli` for provider-specific user workflows when available.
4. Use `agentctl` for provider-neutral orchestration and diagnostics.

## Validation

- `cargo test -p nils-agentctl --test workflow_run`
- `cargo test -p nils-agentctl --test provider_commands`
