# Session display metadata v1

Display metadata is stored in the canonical session document. It describes a
session and never grants permissions, changes a forge principal, or rewrites
lineage, adoption, runtime identity or work references.

The existing `--role ROLE` launch identifier remains explicit, bounded and
non-inherited. Display consumers recognize `coordinator`, `domain-coordinator`,
`steward`, `reviewer`, `tester`, `analyst`, `ideas` and `local-model` independently
of titles and provider models. Both coordinator roles require a root at launch.
Unknown display roles must preserve the session and use the consumer's ordinary
lineage fallback, without guessing a specialist identity.

`start --title-mode auto|pinned` and HTTP `POST /sessions` accept a title mode.
The default is `auto`, including sessions created before this contract. CLI
console starts forward pinned mode and require its persisted mode/revision in
the response. An unconfirmed pin returns the created session ID and forbids a
blind duplicate-start retry. Auto remains compatible with older Console readers. Fresh sessions and provider-resume imports
persist it; ordinary restart/resume and archive/history resume preserve it.
Archives written before this field resume in auto mode. History resume starts a
new creation identity with display revision zero. Lists expose `title_mode` and
`display_revision` (initially zero). A pinned session retains its title and
structured title state; automatic and manual model retitle cannot overwrite it.
Explicit operator title edits remain available through the existing title API.

## Updating an existing session

Authenticated `POST /sessions/{exact-id}/display-metadata` accepts only:

```json
{
  "expected_session_created_at": "2026-10-01T00:00:00Z",
  "expected_revision": 0,
  "role": "reviewer",
  "title_mode": "pinned"
}
```

At least one of `role` and `title_mode` is required. Role is an optional bounded
identifier; omit it to keep the current role. Mode accepts only `auto` or
`pinned`. The route uses the daemon's ordinary owner credential, never a managed
child capability. IDs are exact; unique prefixes do not select a record. The
creation identity and metadata revision are checked under the lifecycle record
lock. Conflict returns `display-revision-conflict` (HTTP 409) without writing.
A successful update returns the ordinary session view in the authenticated
`cli.agent-session.serve.v1` envelope, including the persisted mode, role and
revision. An unchanged value at the current revision is a no-op.

A session with an immutable `lineage.forge_context` keeps its launch role.
Changing that role, including assigning a role when the binding has none,
returns `display-role-bound` (HTTP 409) without writing any part of the request.
Start a new session to use another bound role. Requests retaining the same role
and title-mode-only updates remain available and preserve the forge context.

A changed mode also advances `title_revision`, invalidating every admitted v2/v3
retitle result even if the operator subsequently re-enables auto. Retitle checks
pinned mode before admission and again under its commit authority; failure is
`title-mode-pinned` (HTTP 409). The daemon's automatic scanner skips pinned
sessions. No provider, account selection or fallback configuration changes.

Consumers must deploy compatible role readers before enabling new producer
values. A missing mode/revision projection means the daemon does not support the
mode control; clients must not claim a local setting is persisted remotely.

## Managed CLI title writers

```bash
agent-session title set SESSION "Review the parser" --if-revision 0 --format json
agent-session title auto SESSION --if-revision 1 --format json
```

Both commands require a current ordinary runtime capability, supplied by
`--capability-file` or `AGENT_SESSION_CAPABILITY_FILE`. There is no unauthenticated
operator bypass on this CLI surface. The capability identifies the writer;
an environment session selector alone is insufficient. Scoped worker credentials
do not grant title access. The target ID is exact, and `--if-revision` is required
and compares the target's `title_revision` under its lifecycle record lock.

The permitted writers are the target itself, its recorded launch parent or root
coordinator, and its currently recorded `lineage_adoption.adopted_by` steward.
A supervising lead that is not the launch parent uses that existing adoption
record. References match the writer's machine, exact ID and session creation
identity; a replaced session does not inherit title access. Role labels,
workspace membership and shared work references grant no access. This rule is
specific to title updates and adds no grant mechanism or broader lineage power.
Every other session receives `title-forbidden` (data/65) without a write.

`set` trims the supplied title, requires a nonempty single line of at most 120
Unicode characters, and rejects control characters and Unicode line separators.
It uses the same plain-title validation as session creation. It stores the title,
clears any previous structured topic, and sets `title_mode` to `pinned` atomically.
`auto` sets the mode to `auto` and clears structured topic ownership, leaving the
current display title until the next generated topic replaces it. Neither command
invokes a provider or sends input to a running pane. Existing Console title-edit
and retitle routes are unchanged.

Each successful command advances `title_revision` and `display_revision`, fencing
in-flight retitle and display-metadata requests. Stale title revisions return
`title-revision-conflict` (data/65), with the current revision and typed recovery
guidance, without writing. Validation failures use `title-invalid` or
`title-too-long` (usage/64). Refusals never echo the submitted title or capability.
Success uses `cli.agent-session.title-set.v1` or
`cli.agent-session.title-auto.v1` and returns the ID, title, title mode and both
revisions. Board, list and Console consume the canonical record through the
same display projection as a start-time pinned title.

## On-demand refresh

`agent-session refresh ID` and `POST /sessions/{exact-id}/refresh` (bearer, empty
body) re-read the facts a session displays from live provider state. They run
only when asked; nothing polls.

- `model`: the latest model in the provider transcript (Claude) or rollout
  turn context (Codex), read from a bounded tail window.
- `reasoning_effort`: the latest rollout turn-context effort (Codex). Claude
  transcripts record no effort, so a Claude model change clears the stored
  effort rather than guessing.
- `title`: a structured, unpinned title is re-rendered with `references` taken
  from the session's current work issues (at most two, as `repo #N`). Pinned
  titles, plain titles and sessions without work are left alone. A change
  advances `title_revision`.

The response lists the fields that `changed`, the fields that could not be
re-detected in `unavailable`, and the refreshed session view as `session`. The
provider account is always `unavailable`: the account brokers are token-free and
report no signed-in identity (tracked in sympoies/nils-cli#3029), so the response carries the current
`codex_account` / `claude_account` views unchanged. A refresh with nothing new
does not rewrite the record. Sessions other than Claude and Codex fail with
`refresh-unsupported-agent`. The serve envelope is
`{"machine": ..., "refresh": {...}}`.
