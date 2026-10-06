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

A changed mode also advances `title_revision`, invalidating every admitted v2/v3
retitle result even if the operator subsequently re-enables auto. Retitle checks
pinned mode before admission and again under its commit authority; failure is
`title-mode-pinned` (HTTP 409). The daemon's automatic scanner skips pinned
sessions. No provider, account selection or fallback configuration changes.

Consumers must deploy compatible role readers before enabling new producer
values. A missing mode/revision projection means the daemon does not support the
mode control; clients must not claim a local setting is persisted remotely.
