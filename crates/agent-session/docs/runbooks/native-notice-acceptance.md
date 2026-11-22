# Disposable native notice acceptance

Run this profile from the repository root to check an installed notice producer
and history reader against real Claude Code and Codex CLI runtimes. The profile
uses a fixed-response loopback inference server, so it needs no provider account
or network access.

## Prerequisites

Use a Linux host with working unprivileged user, PID, and network namespaces,
Bubblewrap (`bwrap`), Python 3.9 or newer at `/usr/bin/python3`, and system tmux
3.4 or newer. Supply the native executables below. For Codex, use the platform
binary from the pinned release or npm package rather than its JavaScript launcher.

| Executable | Version |
| --- | --- |
| `agent-session` and its sibling `agent-hook` | 1.42.0 |
| Claude Code | 2.1.317 |
| Codex CLI | 0.162.1 |

The pins describe this acceptance profile. The
[session notice contract](../specs/session-notices-v1.md) defines runtime support.

## Run the profile

Dry-run requires only Python and creates zero resources.

```bash
python3 -I crates/agent-session/tests/fixtures/native-notice-acceptance.py --dry-run
```

It reports one private state root, two provider homes, one tmux server, one
serve daemon, one inference server, two managed sessions, two native notices,
and two typed lookalikes.

Set `AGENT_SESSION_BIN`, `CLAUDE_BIN`, and `CODEX_BIN` to the absolute paths of the
pinned binaries. Then run:

```bash
python3 -I crates/agent-session/tests/fixtures/native-notice-acceptance.py \
  --agent-session "$AGENT_SESSION_BIN" --claude "$CLAUDE_BIN" --codex "$CODEX_BIN" \
  --enable-writers
```

The helper clears the inherited environment and mounts the four binaries
read-only in a sandbox with private homes, runtime sockets, and loopback.
Each session first completes an ordinary synthetic turn to create its
provider-owned transcript. The helper waits up to 18 seconds for each idle
runtime and seed history row. It calls `agent-session notice` once per harness
with runtime and provider identity guards and an idempotency key. The existing
notice command enforces the idle and empty-composer guards.

After observing the notice turn, the helper uses ordinary TUI input to submit a
single lookalike. It queries the installed history endpoint and checks the exact
envelopes in memory. Each harness must return one `role: notice`,
`row_class: notice` row followed by one `role: user` row without a row class.
The JSON report contains the native transport and row classification counts.
Provider and history payloads stay inside the sandbox.

The whole run is bounded to four minutes. Exit 0 with `PASS` means both
harnesses passed and cleanup was verified. Exit 2 with `UNRUN` gives a typed
refusal for an unsupported pin, disabled rollout, unready runtime, or
unconfirmed notice attempt. The helper never replays that attempt or switches
to terminal notice delivery. Omit `--enable-writers` to exercise the installed
writer's `notice-writers-disabled` refusal without launching sessions. Exit 1
with `FAIL` reports a failed history check or fixture failure.

## Cleanup and contract tests

After collecting the metadata, the helper runs `agent-session delete
--leave-processes` for each fixture session. This uses the installed CLI's
verified tmux and session-state teardown in the private namespace. The helper
checks that `agent-session list` is empty, stops the private tmux server if it
is still running, and verifies its socket has no listener. On exit,
Bubblewrap tears down the private PID namespace and its remaining provider
processes, and discards the temporary state and homes.

With the same binary paths exported, run the contract checks:

```bash
python3 -I crates/agent-session/tests/fixtures/test_native_notice_acceptance.py -v
```
