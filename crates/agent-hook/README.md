# agent-hook

`agent-hook` is the shared policy control plane for Codex and Claude native
hooks and the out-of-tree DeepSeek Harness (DSH) runtime bundle. Provider
configuration contains only one dispatcher command per required event/matcher
group; versioned policy, deterministic aggregation, diagnostics, and governed
recovery live behind that ingress.

## Package and binary

| Field | Value |
| --- | --- |
| Package | `nils-agent-hook` |
| Binary | `agent-hook` |

## Configuration

The default user config is
`${XDG_CONFIG_HOME:-$HOME/.config}/agent-hook/config.toml`. It selects one
absolute, digest-pinned policy bundle:

```toml
schema_version = "agent-hook.config.v1"

[policy]
path = "/absolute/path/to/agent-hook/policies/current/policy.toml"
digest = "sha256:<64-lowercase-hex>"
```

Config and policy files are strict TOML. Unknown fields, untrusted paths,
unsupported capability IDs, invalid override authority, and digest drift are
rejected.

## Common commands

```bash
agent-hook validate --format json
agent-hook doctor --all --format json
agent-hook setup --product codex --dry-run --format json
```

## Documentation

| Need | Document |
| --- | --- |
| Commands, setup, DSH integration and recovery | [Command guide](docs/runbooks/commands.md) |
| Schemas, policy registry and contract map | [Agent-hook v1](docs/specs/agent-hook-v1.md) |
| Every crate document by purpose | [Documentation index](docs/README.md) |
