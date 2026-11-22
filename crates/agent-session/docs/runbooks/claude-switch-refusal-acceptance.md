# Claude account-switch refusal acceptance

Use this fixture to test an installed consumer against the ordinary authenticated
`PUT /sessions/{id}/account` route. The driver runs an installed `agent-session`
binary and a synthetic Claude process that uses the runtime-provided conversation
ID. It requires no Claude Code installation, provider account, or credentials.

## Prerequisites

Use Linux with a systemd user manager, cgroup v2, Linux pidfds, Python 3.9 or
newer, and tmux. The tested runtime versions are `1.41.10` and `1.42.0`. The
driver checks for an `agent-session` 1.x binary at version `1.41.10` or newer
and uses Claude account broker v2. The synthetic provider has no Claude Code
version dependency.

## Run the fixture

From the repository root, set `AGENT_SESSION_BIN` to the installed executable.
Preview the exact resource counts before running it.

```bash
python3 -I crates/agent-session/scripts/claude-switch-refusal-fixture.py \
  --agent-session "$AGENT_SESSION_BIN" --dry-run

python3 -I crates/agent-session/scripts/claude-switch-refusal-fixture.py \
  --agent-session "$AGENT_SESSION_BIN"
```

The dry run reports one private root, daemon, tmux server, session and
synthetic provider process, plus two account nicknames and one local synthetic
account store. A live run creates those resources under a fresh mode-0700 root
or on the daemon's ephemeral loopback port. The runtime receives a clean
environment and uses the fixture's synthetic broker and provider executables.
The only links out of the private runtime directory are the user manager's
`bus` and `systemd/private` sockets.

The driver creates the session through `POST /sessions` with the explicit
`fixture-source` account. It uses the supported activity event command to give
the synthetic session an idle boundary. When the account route asks the
fixture broker to materialize the listed `fixture-target` account, the broker
returns a fixed refusal. The runtime responds with HTTP 422,
`claude-account-switch-refused`, and cause `claude-account-broker-rejected`.

A `PASS` receipt reports that before and after the request, the live pane,
provider process and conversation, runtime incarnation, selected account and
revision, and empty pending intent were unchanged. The synthetic account
inventory and store metadata must also match. The driver bounds its commands
and HTTP requests and runs `agent-session delete` for the owned session before
returning. The final receipt verifies absence of the daemon, session, tmux
server, provider process, and private root. `UNRUN` exits 2 with a typed
prerequisite reason. A failed assertion or incomplete cleanup exits 1. If the
runtime's deletion guards refuse cleanup, retain the reported private recovery
root for the owner.

## Exercise an installed consumer

Append a foreground consumer command to the live invocation.

```bash
python3 -I crates/agent-session/scripts/claude-switch-refusal-fixture.py \
  --agent-session "$AGENT_SESSION_BIN" -- /absolute/path/to/consumer-check
```

The driver checks the route once, then gives the consumer up to 30 seconds to
attempt one additional switch against the same running session. The consumer
reads the synthetic bearer from the file in
`AGENT_SESSION_FIXTURE_TOKEN_FILE`. Use `AGENT_SESSION_FIXTURE_URL`,
`AGENT_SESSION_FIXTURE_SESSION_ID`, `AGENT_SESSION_FIXTURE_INCARNATION`, and
`AGENT_SESSION_FIXTURE_TARGET_ACCOUNT` to configure the client and its
request. The command runs with a clean environment and private working
directory. Keep the bearer private and end any consumer-owned children before
the command exits. An additional target preflight attempt and a zero consumer
exit are required for success. The driver emits only the metadata receipt.

Run the fixture contract tests with the same installed binary.

```bash
CLAUDE_SWITCH_TEST_AGENT_SESSION="$AGENT_SESSION_BIN" \
  python3 -I crates/agent-session/scripts/test_claude_switch_refusal_fixture.py -v
```
