# agentctl

## Overview

`agentctl` is the provider-neutral control plane for local agent operations.

It owns:

- provider registry/selection (`provider`)
- provider-neutral diagnostics (`diag`)
- debug bundles (`debug`)
- declarative orchestration (`workflow`)
- local automation integrations (`automation`)

## Command ownership boundary

| Job | Primary owner |
|---|---|
| OpenAI/Codex auth, Codex prompt wrappers, Codex rate-limit diagnostics, Starship | `codex-cli` |
| Multi-provider registry/selection (`provider`), provider-neutral doctor/debug/workflow | `agentctl` |
| Local automation tool orchestration (`macos-agent`, `screen-record`, `image-processing`, `fzf-cli`) | `agentctl` |
| Provider adapter implementation against `provider-adapter.v1` | `agent-provider-*` crates + `agent-runtime-core` |

- `agentctl` owns provider-neutral orchestration (`provider`, `diag`, `debug`, `workflow`, `automation`) and local automation integration.
- `codex-cli` remains responsible for provider-specific OpenAI/Codex operations.
- Migration note: keep existing `codex-cli` workflows stable while provider-neutral ownership lives in `agentctl`.
- Compatibility shim: `wrappers/codex-cli` forwards `provider|debug|workflow|automation` to `agentctl` when `agentctl` is available.
- Migration hint text (wrapper/help/docs): `use agentctl <command> for provider-neutral orchestration`.

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
cargo run -p nils-agentctl -- provider list --format json
```

Healthcheck one provider:

```bash
cargo run -p nils-agentctl -- provider healthcheck --provider codex --format json
```

Override selection:

- CLI override: `--provider <id>`
- Environment override: `AGENTCTL_PROVIDER=<id>`

## Future provider onboarding

For new providers, follow:

- `../agent-runtime-core/README.md`
- `../../docs/runbooks/provider-onboarding.md`

Required minimum:

1. Create `crates/agent-provider-<provider>`.
2. Implement `ProviderAdapterV1` skeleton.
3. Register adapter in `ProviderRegistry::with_builtins()`.
4. Add contract tests and run validation commands.

## Docs

- [Docs index](docs/README.md)
