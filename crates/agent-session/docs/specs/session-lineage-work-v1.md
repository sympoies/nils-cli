# Session Lineage And Work V1

## Status and ownership

- Status: implementation contract for `agent-session` session lineage and
  work references.
- Schema: `agent-session.session-lineage.v1` for the `lineage` object; `work`
  is an unversioned member of `agent-session.session.v1`.
- Owner: `nils-agent-session`.
- Compatibility: additive to `agent-session.session.v1`. Both members are
  optional typed fields of `sessions/<id>/session.json`, so older readers keep
  them through the record's unknown-key passthrough, and a record created
  before this contract has neither.
- Design: sympoies/nils-cli#2032, sections 1 and 2.

Every managed session records **who started it** (`lineage`) and **which
program and issues it works on** (`work`). Readers such as `agent-session
list`, the serve session view, the session board, and dispatchers read them
from the shared record instead of keeping their own registries.

Lineage is descriptive. It never authorizes anything, the same rule as Agent
Console's dispatcher mark: knowing a session's parent grants no access to
either session.

## Session references

A session is identified across machines by
`(machine, session_id, session_created_at)`:

```json
{"machine": "sympoies", "session_id": "85a7…", "session_created_at": "2026-10-01T05:25:45.981481421Z"}
```

- `machine` is the label of the machine the session runs on, resolved the same
  way the board and `list` resolve it: `serve --machine`, then
  `AGENT_SESSION_MACHINE`, then `--host` / `AGENT_SESSION_HOST`, then the short
  hostname. On a console start it is the caller daemon's federation machine.
- A `parent` reference may also carry `session_incarnation`, the parent
  runtime's `launch_id` when the child was started. It is kept for audit only
  and never takes part in a match, so a parent that restarts or switches
  accounts (same id, new `launch_id`) keeps its children.
- A `root` reference never carries an incarnation.

## `lineage`

Written once when the record is created; it does not change afterwards.

```json
"lineage": {
  "schema_version": "agent-session.session-lineage.v1",
  "parent": {"machine": "sympoies", "session_id": "85a7…", "session_created_at": "…", "session_incarnation": "…"},
  "root":   {"machine": "sympoies", "session_id": "3f1c…", "session_created_at": "…"},
  "depth": 2,
  "starter": {"kind": "session", "via": "console"},
  "budget": null
}
```

| Field | Meaning |
| --- | --- |
| `parent` | The session that started this one, or `null` for a root. |
| `root` | The topmost ancestor. A root names itself. |
| `depth` | `0` for a root, otherwise the parent's depth plus one; at most 64. |
| `starter.kind` | `session` (started from inside a managed session), `main-agent` (a Main Agent worker; the `orchestration` projection stays authoritative for roles), `console` (a browser create in Agent Console), or `operator` (a shell or HTTP caller with no managed parent, and every intentional new root). |
| `starter.via` | `cli`, `console` (relayed through Agent Console), or `http` (a direct `POST /sessions`). |
| `budget` | Reserved for the subtree budget of the admission contract; always `null` in v1. |

`root` and `depth` are stored, not derived. Ancestors can be deleted, leave the
closed-session ledger, or run on an unreachable host, so walking the chain at
read time is unreliable. Readers derive the tree from `parent` edges and group
by `root`.

A session with `starter.kind` `session` or `main-agent` always has a parent; a
`console` or `operator` session never does.

### Plain CLI start and run

`agent-session start` and `agent-session run`:

1. Outside a managed session (`AGENT_SESSION_ID` unset), or with
   `start --no-parent`, the new session is a root with
   `starter {"kind": "operator", "via": "cli"}`.
2. Inside a managed session, the command resolves `AGENT_SESSION_ID` in its
   own state directory and requires `AGENT_SESSION_RUNTIME_ID` to equal that
   session's current `launch_id`. On success the caller is the parent: `root`
   is the parent's `root` (or the parent itself when the parent has no
   lineage), `depth` is the parent's plus one, and
   `starter {"kind": "session", "via": "cli"}`.
3. When the id does not resolve (for example after `ssh` to another host, or
   with a foreign state directory) or the runtime id does not match, the
   command never guesses. It records an operator root and prints one line to
   stderr: `warning: AGENT_SESSION_ID <id> … starting a new root session
   without a parent`. The JSON on stdout is unchanged.

A start whose parent is already 64 deep fails with `lineage-depth-exceeded`
before anything is created.

### Serve create

`POST /sessions` accepts an optional `lineage` object, the caller's statement of
who started the session. The daemon bearer is already trusted, so the daemon
stores it after validating its shape:

```json
{
  "schema_version": "agent-session.session-lineage.v1",
  "parent": {"machine": "…", "session_id": "…", "session_created_at": "…", "session_incarnation": "…"},
  "root": {"machine": "…", "session_id": "…", "session_created_at": "…"},
  "depth": 2,
  "starter": {"kind": "session", "via": "console"}
}
```

- `schema_version` is optional; when present it must be
  `agent-session.session-lineage.v1`.
- A `session` or `main-agent` start names `parent` and `root` with `depth`
  1 to 64. A `console` or `operator` start names neither, with `depth` 0; the
  daemon then makes the session its own root.
- `machine` is 1 to 64 printable ASCII bytes, `session_id` a valid session id,
  `session_created_at` an RFC 3339 timestamp, and `session_incarnation` 1 to
  128 printable ASCII bytes. An incarnation on `root` is dropped.
- Unknown keys, an unknown `starter.kind` or `starter.via`, or an inconsistent
  shape fail with HTTP 400 `lineage-invalid` (a `depth` over 64:
  `lineage-depth-exceeded`) before anything is created.
- Without `lineage` the session is a root with
  `starter {"kind": "operator", "via": "http"}`. A session restored from
  provider history is created the same way.

The create response's `session` echoes the stored `lineage` and `work`. A
caller that sent `lineage` and gets no `session.lineage` back is talking to a
daemon older than this contract.

### Console starts

`agent-session start --via-console` relays through the caller's daemon (see
[Owned child sessions](session-coordination-v1.md#owned-child-sessions-v1)).
The daemon route has already authenticated the caller's exact incarnation, so
it computes the child's lineage from the caller's own record:

- By default the caller is the parent:
  `parent = (federation machine, caller id, caller created_at, caller
  launch_id)`, `root` and `depth` as for a CLI start, and
  `starter {"kind": "session", "via": "console"}`.
- With `--no-parent` the child is a new root:
  `parent: null, root: null, depth: 0,
  starter {"kind": "operator", "via": "console"}`. The target daemon names the
  child as its own root.

The daemon puts the result in the relayed `session` object as `lineage`, in the
create-body shape above, together with the resolved `work`. A `lineage` or
`work` the caller put in `session` itself is replaced. The aggregator checks
`lineage.parent` against `(relay machine, source_session_id,
source_incarnation)` and forwards both members in the target daemon's
`POST /sessions` body. An aggregator that predates this contract drops unknown
`session` members, so the child is then created as an HTTP operator root and
the aggregator's own dispatcher mark stays the only parent link.

### Main Agent workers

`main-agent worker start` records the Run owner session as the worker's parent
with `starter {"kind": "main-agent", "via": "cli"}` and the owner's work
inherited. A worker whose owner cannot be recorded (for example a chain already
64 deep) still starts, without lineage.

## `work`

```json
"work": {
  "program": {"provider": "github", "repository": "serenvia/laoda", "number": 44},
  "issues": [{"provider": "github", "repository": "sympoies/nils-cli", "number": 2032}],
  "inherited": true,
  "revision": 1
}
```

| Field | Meaning |
| --- | --- |
| `program` | The work-mode program tracker issue, or `null`. At most one. |
| `issues` | The issues this session works on, sorted and distinct; at most 4. |
| `inherited` | `true` when the start named neither a program nor issues and both came from the parent. |
| `revision` | `1` when created. |

A session with neither a program nor issues has no `work` member.

### References

A reference is `[provider:]owner/repo#N` on the command line and
`{"provider", "repository", "number"}` in JSON:

- `provider` is `github` (the default) or `gitlab`.
- `repository` is a canonical lowercase `owner/name` (exactly two
  components).
- `number` is a positive decimal integer.

Only public references are accepted. Free text, whitespace, a missing number,
or more than 4 issues fail with `work-ref-invalid` (usage exit class; HTTP 400
on serve) before anything is created.

### Start flags and inheritance

- `--program <owner/repo#N>`: at most one.
- `--issue <owner/repo#N>`: repeatable, at most 4.
- `--no-inherit-work`: take nothing from the parent.

By default a child copies its parent's `program` and `issues`. An explicit
`--program` or `--issue` replaces that dimension only; the other is still
inherited. With `--no-parent` there is no parent to inherit from. The same flags
apply to `start --via-console`, where the caller's daemon resolves inheritance
from the caller's record and sends the resolved `work`.

`POST /sessions` accepts the resolved `{"program", "issues", "inherited"}`
object as `work` and stores it with `revision: 1`; unknown keys fail with
`work-ref-invalid`.

`work` is separate from `work-context`. A work context is a short-lived
collision claim; `work` is the session's durable statement of what it belongs
to.

## Read surfaces

The session view (`agent-session list --format json`, `GET /sessions`, and the
start and create results) carries the stored `lineage` and `work` objects
unchanged. Both are absent on records that have none.

## Failure codes

| Code | Exit class / HTTP | When |
| --- | --- | --- |
| `work-ref-invalid` | usage / 400 | A reference outside the grammar, more than 4 issues, or an invalid `work` object. |
| `lineage-invalid` | usage / 400 | A create body `lineage` with an invalid shape. |
| `lineage-depth-exceeded` | usage / 400 | A start deeper than 64. |
