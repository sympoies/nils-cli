# codex-cli to Claude mapping

## Purpose

Help operators migrate codex-oriented workflows to Claude-backed provider workflows in `agentctl`.

## Ownership boundary alignment (core vs CLI)

This runbook captures CLI-facing mapping and migration guidance. The provider contract,
error mapping, and execution policy remain core-owned in
`crates/agent-provider-claude/docs/specs/claude-provider-contract-v1.md`.

CLI ownership in this runbook:

- provider command UX mapping from codex-style commands to `agentctl` commands
- `diag` command/operator workflow guidance and output expectations
- `workflow` reporting expectations for migration outcomes and unsupported surface handling
- root help/completion expectations used by wrappers and operator docs

Anti-goals:

- anti-goal: do not redefine provider contract semantics that are owned by core docs/modules
- anti-goal: do not require provider core modules to emit CLI-rendered text/tables for `diag` or `workflow` output

## Command-family mapping (parity aligned)

Source of truth for classifications (`exact`, `semantic`, `unsupported`):
`crates/agent-provider-claude/docs/specs/codex-cli-claude-parity-matrix-v1.md`.

| Command family | codex-cli surface | Claude path in this repo | Class | Notes |
| --- | --- | --- | --- | --- |
| `agent` | `codex-cli agent prompt ...` | `agentctl workflow run` provider step (`provider=claude`) | `semantic` | Prompt intent is preserved; envelope/wording are provider-neutral. |
| `agent` | `codex-cli agent advice ...` | same + `advice:` prefixed prompt intent | `semantic` | Uses advice template expansion in Claude adapter flow. |
| `agent` | `codex-cli agent knowledge ...` | same + `knowledge:` prefixed prompt intent | `semantic` | Uses explanatory template expansion in Claude adapter flow. |
| `auth` | `codex-cli auth current` | provider `auth-state` surfaced by `agentctl provider healthcheck --provider claude --format json` | `exact` | Read-only auth intent maps directly. |
| `auth` | `codex-cli auth login` | configure `ANTHROPIC_API_KEY`, then validate with `agentctl diag doctor --provider claude --format json` | `semantic` | Equivalent operator goal, different login mechanism. |
| `diag` | `codex-cli diag doctor` | `agentctl diag doctor --provider claude` | `semantic` | Readiness intent is equivalent with provider-neutral output shape. |
| `diag` | `codex-cli diag rate-limits` | provider execute error category `rate-limit` + `diag doctor` readiness context | `semantic` | No Codex rate-limit table; rate-limit state is surfaced in execute diagnostics. |
| `config` | `codex-cli config show` | inspect `ANTHROPIC_*` / `CLAUDE_*` environment config | `semantic` | Same operator intent, env-driven contract. |
| `config` | `codex-cli config set` | set environment variables consumed by Claude adapter | `semantic` | Same operator intent, no shell-snippet emitter. |

## Unsupported behavior alternatives

| codex-cli surface | Class | Why unsupported | Explicit alternative |
| --- | --- | --- | --- |
| `codex-cli agent commit` | `unsupported` | Commit workflow is codex/git-specific and outside provider adapter contract. | Run git commit workflow directly, then run `agentctl workflow run` for Claude-backed execution tasks. |
| `codex-cli auth use/save/remove/refresh/auto-refresh/sync` | `unsupported` | Claude adapter has no Codex profile/secret-store lifecycle surface. | Manage credentials through env/secret manager, then validate via `agentctl provider healthcheck --provider claude --format json` and `agentctl diag doctor --provider claude --format json`. |
| `codex-cli starship` | `unsupported` | Starship integration is Codex-specific UI behavior. | Use `agentctl provider|diag|workflow` commands directly (or wrapper hint to `agentctl`), and optionally export shell completion via `agentctl completion <bash|zsh>`. |

## Fallback policy

- Unsupported codex-cli surfaces require explicit alternatives and must not silently fallback.
- Wrapper/help text should direct operators to the mapped `agentctl` command family or this runbook.

## Help and completion expectations

- `agentctl --help` is the canonical root surface and should list: `provider`, `diag`, `debug`, `workflow`, `automation`, `completion`.
- `agentctl completion <shell>` is the canonical completion export path and currently supports `bash` and `zsh`.
- Help/wrapper copy for codex migration must preserve explicit unsupported alternatives (no silent fallback).

## Recommended migration steps

1. Set `ANTHROPIC_API_KEY`.
2. Validate readiness:
   - `cargo run -p nils-agentctl -- provider healthcheck --provider claude --format json`
   - `cargo run -p nils-agentctl -- diag doctor --provider claude --format json`
3. Follow command-family mappings above (including unsupported alternatives with no silent fallback).
4. Run workflow with Claude provider steps.

## Validation

- `cargo test -p nils-agentctl --test dispatch`
- `cargo test -p nils-agentctl --test workflow_run`
- `cargo test -p nils-agentctl --test provider_commands`
