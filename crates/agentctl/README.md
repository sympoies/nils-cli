# agentctl

## Overview

`agentctl` is the provider-neutral control plane for local agent operations.

It owns:

- provider registry/selection (`provider`)
- provider-neutral diagnostics (`diag`)
- debug bundles (`debug`)
- declarative orchestration (`workflow`)
- local automation integrations (`automation`)

## Ownership boundary

| Concern | Owner |
|---|---|
| OpenAI/Codex-specific auth + execution wrappers | `codex-cli` |
| Provider-neutral orchestration and multi-provider contracts | `agentctl` |
| Adapter implementations (`codex`, `claude`, `gemini`) | `agent-provider-*` crates |

If you need provider-neutral orchestration, use `agentctl`.
If you need Codex-specific auth/prompt tooling, use `codex-cli`.

## Usage

```text
Usage:
  agentctl <group> <command> [args]

Groups:
  provider   list | healthcheck
  diag       capabilities | doctor
  debug      bundle
  workflow   run
  automation (reserved)
```

## Provider registry

Built-in providers:

- `codex` (`maturity=stable`)
- `claude` (`maturity=stub`)
- `gemini` (`maturity=stub`)

List providers:

```bash
cargo run -p agentctl -- provider list --format json
```

Healthcheck one provider:

```bash
cargo run -p agentctl -- provider healthcheck --provider codex --format json
```

Override selection:

- CLI override: `--provider <id>`
- Environment override: `AGENTCTL_PROVIDER=<id>`

## Future provider onboarding

For new providers, follow:

- `../../docs/runbooks/provider-onboarding.md`

Required minimum:

1. Create `crates/agent-provider-<provider>`.
2. Implement `ProviderAdapterV1` skeleton.
3. Register adapter in `ProviderRegistry::with_builtins()`.
4. Add contract tests and run validation commands.
