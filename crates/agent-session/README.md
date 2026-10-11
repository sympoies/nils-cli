# agent-session

`agent-session` starts and manages tmux-backed Codex, Claude Code, and DSH
sessions for mobile handoff workflows. It is designed for personal automation
such as the agent-console mobile control plane: a service can create the
session with a full prompt, then return a short tmux attach command for the
user to continue from Termius, glance at the pane, or steer it with keystrokes.

## Package vs binary name

| Field        | Value                |
| ------------ | -------------------- |
| Package name | `nils-agent-session` |
| Binary name  | `agent-session`      |

## Quick start

Each group below lists common commands and links the document that owns their
behavior. Run `agent-session <command> --help` for every flag.

### Start and observe sessions

```bash
agent-session start --agent codex --cwd ~/Project/foo --prompt-file prompt.md
agent-session start --agent claude --issue sympoies/nils-cli#2032   # child of this session; inherits its program
agent-session list
agent-session board --state live --since 3d --format json   # who else is working (session board v1)
agent-session glance <id> --tail 40
agent-session command <id>
agent-session attach <id>
agent-session logs <id>
agent-session completion zsh
```

Lineage and program/issue references are defined in
[Session lineage and work v1](docs/specs/session-lineage-work-v1.md); the
cross-host board is defined in [Session board v1](docs/specs/session-board-v1.md).

### Drive a live session

```bash
agent-session send <id> --text yes --key enter
agent-session send <id> --key c-c
agent-session send <id> --key down --key enter   # answer a dialog while blocked
agent-session send <id> --text "custom answer" --allow-blocked
agent-session clear <id> --expect-idle --format json
agent-session rebind <id> --format json
agent-session resume <id>
agent-session account show <id> --format json
agent-session account switch <id> --account <nickname> --format json
agent-session delete <id>
```

[Session lifecycle operations](docs/runbooks/session-lifecycle.md) covers
prompt submission outcomes, blocked input, clear and rebind, resume, verified
deletion, startup logs, DSH launch, account switching, model settings, and
secret safety.

### Coordinate work and exchange mail

```bash
agent-session work set <id> --issue sympoies/nils-cli#2040 --if-revision 1
agent-session lineage adopt <child> --by <steward>   # successor takes over a child
agent-session work-context status --format json
agent-session work-context set --tier issue --issue 123 --summary "Implement the tracked fix"
agent-session work-context advise --format json
agent-session work-context acknowledge --for 30m
agent-session work-context clear
agent-session readiness --format json
agent-session message inbox --session <id> --category progress --category handoff
agent-session message forward --session <id> --message <message-id> --if-revision 1 --to <destination> --category progress --idempotency-key forward-001
agent-session message audit --older-than 300 --format json
```

Coordination is advisory by default and does not grant or revoke user
authorization, repository permission, provider consent, or workflow authority.
Only a launch with `--coordination-mode enforce` turns claims, admission, and
physical checkout leases into mutation requirements. See [Work coordination](docs/runbooks/work-coordination.md)
and [Session coordination v1](docs/specs/session-coordination-v1.md).

`readiness` authenticates the current managed runtime and verifies its exact
runtime-issued checkpoint file. It does not acquire a claim or authorize
mutations; see
[managed-session readiness](docs/specs/session-coordination-v1.md#managed-session-readiness).
`message audit` reads mailbox metadata without message bodies; see the
[mail audit contract](docs/specs/mail-audit-v1.md).

### Turn state and public metadata

```bash
agent-session activity status <id> --format json
agent-session activity doctor --format json
agent-session activity setup --agent codex --dry-run
agent-session activity setup --agent codex --repair --dry-run
agent-session activity setup --agent codex --repair --expected-preview-digest sha256:<reviewed-plan-digest>
agent-session metadata attach <id> --request-file metadata.json --if-revision 0 --idempotency-key attach-001 --format json
agent-session metadata show <id> --label acceptance.synthetic --format json
```

Provider hooks project metadata-only lifecycle events into a private,
runtime-bound activity snapshot. Provider registration is owned by
`agent-hook`; `activity setup` is a compatibility forwarder. See the
[turn-state contract](docs/turn-state-contract.md).
`metadata attach` binds one bounded, revision-fenced public label; see
[Session public metadata v1](docs/specs/session-public-metadata-v1.md).

### Serve the control plane

```bash
read -r -s AGENT_SESSION_SERVE_TOKEN
printf '%s' "$AGENT_SESSION_SERVE_TOKEN" | \
  agent-session serve --bind 127.0.0.1:8781 --token-stdin
unset AGENT_SESSION_SERVE_TOKEN
```

Bind to loopback, pass the token on stdin, keep the shell variable unexported,
and expose an authenticated edge rather than the raw port. See
[Serve daemon operations](docs/runbooks/serve-daemon.md) and
[Serve API v1](docs/specs/serve-api-v1.md).

## Output contract

Human-readable text is the default. JSON is opt-in with `--format json` on
command subcommands. JSON output uses the workspace envelope:
`schema_version`, `ok`, `data`, optional `warnings`, and `error` on failure.

## Documentation map

| Need | Document |
| --- | --- |
| Drive one session: input, clear, resume, delete, accounts, secrets | [Session lifecycle operations](docs/runbooks/session-lifecycle.md) |
| Collision awareness and work permissions | [Work coordination](docs/runbooks/work-coordination.md) |
| Deploy and operate the HTTP/WebSocket daemon | [Serve daemon operations](docs/runbooks/serve-daemon.md) |
| Investigate a lifecycle incident | [Lifecycle incident evidence](docs/runbooks/lifecycle-journal.md) |
| HTTP and WebSocket endpoints | [Serve API v1](docs/specs/serve-api-v1.md) |
| Turn phases, provider hooks, activity events | [Turn-state contract](docs/turn-state-contract.md), [Activity stream v1](docs/specs/activity-stream-v1.md) |
| Coordination schemas and failure codes | [Session coordination v1](docs/specs/session-coordination-v1.md) |
| Every crate document by purpose | [agent-session documentation](docs/README.md) |

The canonical agent-facing policy, including how an agent responds to overlap
advice, lives in agent-runtime-kit's
[`session-coordination.md`](https://github.com/sympoies/agent-runtime-kit/blob/main/core/policies/session-coordination.md).
This README and the crate documents define CLI and operator semantics only.
