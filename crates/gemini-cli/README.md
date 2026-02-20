# gemini-cli

## Overview
`gemini-cli` is the Gemini-specific CLI shell in the `nils-cli` workspace.
It currently provides the publish-ready parser topology and shell completion export surface.
Runtime logic will be layered in subsequent tasks using `gemini-core`.

## Usage
```text
Usage:
  gemini-cli <group> [command]

Groups:
  agent
  auth
  diag
  config
  starship
  completion <bash|zsh>
```

## Scope boundary
- Shared Gemini runtime primitives belong to `gemini-core`.
- Provider-neutral orchestration belongs to `agentctl`.
- This crate currently owns command parsing shape and completion export only.

## Exit codes
- `0`: success and help output.
- `64`: usage or argument errors.
- `1`: unexpected internal failure while printing help.

## Docs
- [Docs index](docs/README.md)
