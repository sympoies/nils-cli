# `claude agents --json` fixtures

Sanitized samples of `claude agents --json` (`mixed.json` from Claude Code 2.1.317): a JSON array
with one object per active session. Every session id, pid, name and path is
synthetic.

- `mixed.json`: `idle` and `busy` entries have the shape captured from the
  installed CLI. The `waiting` entries and `waitingFor` follow the recorded
  provider spike (`"permission prompt"`); the unrecognized `waitingFor` and
  the `paused` status are synthetic adapter edge cases.
- `malformed.txt`: output that is not a JSON array.
- `with-state.json`: entries carrying the `state` field documented for
  current Claude Code (`working`, `blocked`, `done`, `failed`, `stopped`),
  with `status` set to values that disagree with it, a process-less entry, and
  an unrecognized `state`. Every entry is synthetic.
