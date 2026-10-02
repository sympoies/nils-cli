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
- Design: sympoies/nils-cli#2032, sections 1 and 2, and its role amendment.

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
- A start inside a managed session labels the parent reference and the child
  with the parent's own `lineage.machine`, the label the parent's creator used,
  even when `serve --machine` differs from the hostname. Only a parent without
  lineage falls back to the starting process's label.
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
  "machine": "c8",
  "parent": {"machine": "sympoies", "session_id": "85a7…", "session_created_at": "…", "session_incarnation": "…"},
  "root":   {"machine": "sympoies", "session_id": "3f1c…", "session_created_at": "…"},
  "depth": 2,
  "starter": {"kind": "session", "via": "console"},
  "budget": null
}
```

| Field | Meaning |
| --- | --- |
| `machine` | The label of the machine this session runs on, as its creator resolved it. |
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
inherited, for tmux workers and DSH external workers alike. A worker whose owner cannot be recorded (for example a chain already
64 deep) still starts, without lineage.

## Adoption

`lineage` stays the historical fact. A later steward, for example a successor
dispatcher that takes over its predecessor's open children, is recorded
separately in `lineage_adoption`:

```json
"lineage_adoption": {
  "adopted_by": {"machine": "sympoies", "session_id": "9c2e…", "session_created_at": "…", "session_incarnation": "…"},
  "revision": 1,
  "updated_at": "…"
}
```

Readers use `effective_parent = lineage_adoption.adopted_by ?? lineage.parent`.

```bash
agent-session lineage adopt <CHILD> --by <SESSION> [--if-revision N]
agent-session lineage adopt <CHILD> --by <SESSION> --by-machine <MACHINE> --by-created-at <TIMESTAMP>
agent-session lineage adopt <CHILD> --clear [--if-revision N]
```

- The command runs on the child's machine and changes only the child's record.
- `--by` names a session in this state directory, recorded with its exact
  identity, its own `lineage.machine` label, and its current incarnation. A
  steward on another machine is named with `--by-machine` and
  `--by-created-at` together; either one alone is a usage error, because a
  remote creation time cannot be looked up locally.
- `--clear` removes the steward, so the original parent is effective again.
- Each change increments `revision`. With `--if-revision N`, a revision other
  than `N` (0 when the child was never adopted) fails with
  `lineage-revision-conflict` and `details.current_revision`.
- Adoption cannot create a loop: when the steward is local, the command
  follows effective parents up from the steward through this state directory
  (at most 64 hops) and fails with `lineage-invalid` if it reaches the child,
  including the child itself. A remote steward ends the walk.
- **Authorization.** Inside a managed session (`AGENT_SESSION_ID` set) the
  caller authenticates with its capability (`coordination-unauthorized`
  otherwise) and may only name itself, through its local record, as the
  steward; naming another steward, any remote steward, or clearing one fails
  with `lineage-adopt-forbidden`. A caller outside any
  managed session is an operator and may do either.
- The result (`cli.agent-session.lineage-adopt.v1`) carries `session_id`, the
  unchanged `lineage`, the new `lineage_adoption`, and `effective_parent`.

## Closing a parent

A parent closes its own children before it reports done. `delete` and archive
enforce it:

- `agent-session delete <ID>`, `DELETE /sessions/{id}`, and
  `POST /sessions/{id}/archive` fail with `session-has-live-children` (data exit
  class; HTTP 409) while any session in this state directory, live or stopped,
  has the closing session as its effective parent. `details.children` lists
  them as `{machine, session_id, session_created_at}` and `details.scope` is
  `local`. Nothing is closed.
- `delete --orphan-children`, `?orphan_children=true`, or the archive body's
  `orphan_children: true` closes the session anyway. The result's
  `children` member, `{"scope": "local", "orphaned": [...]}`, records the
  children the caller acknowledged orphaning; it is present, with an empty
  `orphaned`, on every guarded delete or archive.
- A child matches on `(session_id, session_created_at)` of its effective
  parent. The machine label is not compared, because local children name their
  parent with the label of whichever process started them.
- The check is `local`: children on other machines (started with
  `start --via-console --machine`) are not seen. A global check through the
  board relay follows when board records carry lineage.
- There is no cascade: closing children is a multi-target destructive action
  and stays explicit.
- Main Agent cleanup and orchestration group archive close their own workers
  and are not guarded. Neither are the serve maintenance recovery actions
  (`retry_delete`, `terminate_runtime_then_delete`, `remove_console_record`):
  they finish a close that already started or remove a record whose runtime
  cannot be stopped, and refusing them would strand that recovery.

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
| `revision` | `1` when created; incremented by every `work set`. |

A session created with neither a program nor issues has no `work` member.
After a `work set` the member stays, even when both dimensions are empty, so its
revision keeps fencing later updates.

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

### Updating work

Work moves; lineage does not.

```bash
agent-session work set <ID> [--program R | --clear-program] [--issue R]... [--clear-issues] --if-revision N
```

- `--program` replaces the program and `--issue` (repeatable, at most 4)
  replaces the issues; a dimension that is not named is kept. `--clear-program`
  and `--clear-issues` empty one. At least one of the four is required
  (`work-ref-invalid`).
- `--if-revision` is required: the current revision, or 0 when the session has
  no `work`. Any other value fails with `work-revision-conflict` and
  `details.current_revision`.
- The result sets `inherited: false` and increments `revision`.
- **Authorization.** Inside a managed session the caller authenticates with
  its capability and may only set its own work (`work-set-forbidden`
  otherwise). An operator may set any session's.
- The result (`cli.agent-session.work-set.v1`) carries `session_id` and the
  new `work`.

`work` is separate from `work-context`. A work context is a short-lived
collision claim; `work` is the session's durable statement of what it belongs
to.

## `role`

`SessionRecord.role` is `"coordinator"` or absent (null). It marks a session
the operator or its tooling treats as a coordinator, by explicit statement
rather than by inference from the repository name or tree position. Any
number of sessions may hold it at once, for example during a handoff overlap.

- Written once at start; it never changes and is never inherited: a child of a
  coordinator has no role unless it is started with one.
- `agent-session start --role coordinator` (the only accepted value) sets it.
  It is independent of `--no-parent`: a successor coordinator is started with
  `--no-parent --role coordinator`, and its predecessor's live children are
  moved with `lineage adopt --by <successor>`.
- A console start sends `role` as a top-level request key (`machine`,
  `no_parent`, `work`, `role`, `session`) and the daemon relays it as
  `session.role`, next to `session.lineage`. A `role` the caller put in
  `session` itself is replaced. The aggregator forwards it in the target
  daemon's `POST /sessions` body.
- `POST /sessions` accepts an optional `role`, stored verbatim after
  validation. Any value other than `"coordinator"` fails with HTTP 400
  `role-invalid` before anything is created. The create response's `session`
  echoes it.
- `role` is descriptive, like lineage: it authorizes nothing.

## Read surfaces

The session view (`agent-session list --format json`, `GET /sessions`, and the
start and create results) carries the stored `lineage`, `work`,
`lineage_adoption`, and `role` unchanged. Each is absent on records that have
none; readers treat an absent `role` as null.

## Failure codes

| Code | Exit class / HTTP | When |
| --- | --- | --- |
| `work-ref-invalid` | usage / 400 | A reference outside the grammar, more than 4 issues, or an invalid `work` object. |
| `lineage-invalid` | usage / 400 | A create body `lineage` with an invalid shape. |
| `role-invalid` | usage / 400 | A `role` other than `coordinator`. |
| `lineage-depth-exceeded` | usage / 400 | A start deeper than 64. |
| `lineage-revision-conflict` | data | `lineage adopt --if-revision` does not match. |
| `lineage-adopt-forbidden` | data | A managed session names a steward other than itself, or clears one. |
| `work-revision-conflict` | data | `work set --if-revision` does not match. |
| `work-set-forbidden` | data | A managed session sets another session's work. |
| `session-has-live-children` | data / 409 | A delete or archive of a session that local sessions name as their effective parent, without `--orphan-children`. |
