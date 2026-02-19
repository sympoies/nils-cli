# claude-cli JSON Contract v1

## Purpose

Defines machine-readable JSON output for `claude-cli` surfaces that expose `--format json` or `--json`:
- `auth-state show`
- `diag healthcheck`
- `diag rate-limits` (unsupported envelope)
- `config show`

## Schema versions and command paths

| Surface | `command` | `schema_version` |
|---|---|---|
| auth-state show | `auth-state show` | `claude-cli.auth-state.v1` |
| diag healthcheck | `diag healthcheck` | `claude-cli.diag.v1` |
| diag rate-limits (unsupported) | `diag rate-limits` | `claude-cli.diag.v1` |
| config show | `config show` | `claude-cli.config.v1` |

## Envelope rules

Top-level required keys:
- `schema_version`: string
- `command`: canonical command path
- `ok`: boolean

Success envelope:
- `ok=true`
- `result` object present
- `error` absent

Failure envelope:
- `ok=false`
- `error` object with:
  - `code`: stable machine code
  - `message`: deterministic human-readable guidance
- `result` absent

## Unsupported policy

Codex-only surfaces return a stable unsupported envelope.

`diag rate-limits` failure envelope:
- `schema_version=claude-cli.diag.v1`
- `command=diag rate-limits`
- `ok=false`
- `error.code=unsupported-codex-only-command`
- `error.message` includes migration guidance to `diag healthcheck` or `agentctl diag doctor --provider claude`

## Compatibility

- Additive fields are allowed within v1 schema versions.
- Removing/renaming stable fields or changing their semantics is breaking and requires a new schema version.
- Secret/token values must not be emitted.
