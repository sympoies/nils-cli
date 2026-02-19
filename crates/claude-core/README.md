# claude-core

## Overview

`claude-core` is the runtime-only crate for Claude integrations in this workspace.
It provides shared primitives for env configuration, prompt rendering, HTTP client behavior,
and execution mapping used by `claude-cli` and `agent-provider-claude`.

## Ownership and anti-goals

Owned by this crate:
- Claude runtime configuration parsing (`ANTHROPIC_*`, `CLAUDE_*`)
- Prompt intent rendering for prompt/advice/knowledge execution
- Claude API request/response handling, retry mapping, and error categorization
- Runtime execution helper that maps core outcomes to adapter/CLI-friendly results

Not owned by this crate:
- User-facing CLI argument parsing or help text (`claude-cli`)
- Provider-neutral orchestration UX (`agentctl`)
- `provider-adapter.v1` envelope ownership (`agent-provider-*` + `agent-runtime-core`)

## Docs

- [Docs index](docs/README.md)
