# `claude agents --json` fixtures

Sanitized samples of `claude agents --json` (Claude Code 2.1.317): a JSON array
with one object per active session. Every session id, pid, name and path is
synthetic.

- `mixed.json`: `idle` and `busy` entries have the shape captured from the
  installed CLI. The `waiting` entries and `waitingFor` follow the recorded
  provider spike (`"permission prompt"`); the unrecognized `waitingFor` and
  the `paused` status are synthetic adapter edge cases.
- `malformed.txt`: output that is not a JSON array.
