# codex-cli to Claude mapping

## Purpose

Help operators migrate codex-oriented workflows to Claude-backed provider workflows in `agentctl`.

## Mapping

| codex-cli intent | Claude path in this repo | Notes |
| --- | --- | --- |
| `codex-cli agent prompt ...` | `agentctl workflow run` provider step (`provider=claude`) | Prompt text forwarded to Claude messages API. |
| `codex-cli agent advice ...` | same + `advice:` prefixed prompt intent | Uses advice template expansion. |
| `codex-cli agent knowledge ...` | same + `knowledge:` prefixed prompt intent | Uses explanatory template expansion. |
| `codex-cli diag rate-limits` | provider execute error category `rate-limit` + `diag doctor` readiness | No Codex rate-limit table equivalent. |
| `codex-cli auth *` | provider `auth-state` + environment config | Claude adapter uses `ANTHROPIC_API_KEY`; no Codex secret-file management. |
| `codex-cli config show/set` | environment variables (`ANTHROPIC_*`, `CLAUDE_*`) | Configuration is env-driven, not shell-snippet driven. |
| `codex-cli starship` | no mapping | Codex-specific UX surface; unsupported in Claude adapter. |

## Unsupported surfaces

- `agent commit`
- `starship`
- Codex secret store lifecycle commands (`auth save/remove/sync/use`)

## Recommended migration steps

1. Set `ANTHROPIC_API_KEY`.
2. Validate readiness:
   - `cargo run -p nils-agentctl -- provider healthcheck --provider claude --format json`
   - `cargo run -p nils-agentctl -- diag doctor --provider claude --format json`
3. Run workflow with Claude provider steps.

## Validation

- `cargo test -p nils-agentctl --test workflow_run`
- `cargo test -p nils-agentctl --test provider_commands`
