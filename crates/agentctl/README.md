# agentctl

## Overview

`agentctl` is the provider-neutral control plane for local agent operations.

It owns:

- provider registry/selection (`provider`)
- provider-neutral diagnostics (`diag`)
- debug bundles (`debug`)
- declarative orchestration (`workflow`)
- local automation integrations (`automation`)
- shell completion export (`completion`)

## Command ownership boundary

| Job | Primary owner |
|---|---|
| OpenAI/Codex auth, Codex prompt wrappers, Codex rate-limit diagnostics, Starship | `codex-cli` |
| Multi-provider registry/selection (`provider`), provider-neutral doctor/debug/workflow | `agentctl` |
| Local automation tool orchestration (`macos-agent`, `screen-record`, `image-processing`, `fzf-cli`) | `agentctl` |
| Provider adapter implementation against `provider-adapter.v1` | `agent-provider-*` crates + `agent-runtime-core` |

- `agentctl` owns provider-neutral orchestration (`provider`, `diag`, `debug`, `workflow`, `automation`) plus shell completion export (`completion`).
- `codex-cli` remains responsible for provider-specific OpenAI/Codex operations.
- Migration note: keep existing `codex-cli` workflows stable while provider-neutral ownership lives in `agentctl`.
- Migration classification source (`exact`/`semantic`/`unsupported`): [codex to Claude mapping](docs/runbooks/codex-to-claude-mapping.md).
- Unsupported codex surfaces require explicit alternatives (no silent fallback); see the mapping runbook.
- Compatibility shim: `wrappers/codex-cli` forwards `provider|debug|workflow|automation` to `agentctl` when `agentctl` is available.
- Migration hint text (wrapper/help/docs): `use agentctl <command> for provider-neutral orchestration`.

## Usage

```text
Usage:
  agentctl [COMMAND]

Commands:
  provider    list | healthcheck
  diag        capabilities | doctor
  debug       bundle
  workflow    run
  automation
  completion  bash | zsh
```

## Help and shell completion

```bash
cargo run -p nils-agentctl -- --help
cargo run -p nils-agentctl -- completion zsh
cargo run -p nils-agentctl -- completion bash
```

- `completion` is provider-neutral CLI plumbing; codex-to-Claude workflow mapping remains in the runbook below.

## Provider registry

Built-in providers:

- `codex` (`maturity=stable`)
- `claude` (`maturity=stable`)
- `gemini` (`maturity=stub`)

Provider runtime requirements:

| Provider ID | Maturity | Required runtime requirement | Optional dependency |
|---|---|---|---|
| `codex` | `stable` | `codex` binary for execute flows | None |
| `claude` | `stable` | `ANTHROPIC_API_KEY` plus outbound HTTPS access to Anthropic API | Local `claude` CLI for characterization workflows only |
| `gemini` | `stub` | No runtime requirement yet (stub placeholder adapter) | None |

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
- [codex to Claude mapping](docs/runbooks/codex-to-claude-mapping.md)
