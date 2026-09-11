# Session Public Metadata V1

## Purpose

`agent-session metadata` lets an already-authorized caller attach one bounded
public label to exactly one existing managed session and read it back without
exposing the rest of the private session record. `agent-session` owns the state
mutation. Approval, capability intersection, and workload lifecycle remain
caller responsibilities; a session id or idempotency key grants no authority.

## Commands

```text
agent-session metadata attach ID \
  --request-file PATH \
  --if-revision REVISION \
  --idempotency-key KEY \
  --format json

agent-session metadata show ID [--label LABEL] --format json
```

`ID` is the exact canonical session id. Prefix lookup is not supported on this
mutation surface. The request path is opened as one owner-private regular file
without following symlinks. The raw key is accepted through argv for replay
selection but only its domain-separated SHA-256 digest is retained; responses
never echo it.

## Attachment Request

The only accepted JSON shape is:

```json
{
  "schema_version": "agent-session.metadata-attachment.request.v1",
  "label": "acceptance.synthetic",
  "value": "DSH-METADATA-731"
}
```

The request contract is closed:

- The entire file is at most 1,024 bytes.
- `label` is 1-64 bytes, begins with lowercase ASCII, and otherwise contains
  only lowercase ASCII letters, digits, `.`, `-`, or `_`. V1 admits only the
  reviewed public labels `topic`, `category`, `status`, `priority`, `source`,
  `workflow`, `component`, `stage`, and `acceptance.synthetic`; extending that
  registry is a contract review.
- `value` is 1-256 UTF-8 bytes, has no control or surrounding whitespace, and
  is rejected when it resembles a credential or filesystem path. Values for
  `acceptance.synthetic` use the `DSH-METADATA-<identifier>` grammar; other
  labels accept only lowercase public slugs of at most 64 bytes.
- Unknown or nested fields are rejected, including environment, command,
  executable, argument, script, path, credential, and arbitrary patch fields.
- Labels containing credential or execution vocabulary are rejected.
- One session retains at most eight attachments and eight replay receipts.

The raw `value` is request-only material. It is neither stored in the session
record nor returned by either command. Only the canonical whole-request digest
inside the owner-private replay receipt binds exact idempotent replays; it is
never part of a public projection. This fail-safe boundary prevents a
credential format omitted from the syntactic rejection list from becoming a
persistent public disclosure or public offline-guessing oracle.

The limits are part of v1. Increasing them is additive only when readers remain
bounded; relaxing the closed request shape requires a new request version.

## Revision And Replay Semantics

The public-metadata state starts at revision `0`. A new attachment succeeds
only when `--if-revision` equals the current revision, then advances it exactly
once. Labels are immutable and unique within one session.

The idempotency receipt binds the exact canonical request digest. An exact
replay returns the original attachment identity, resulting revision, and
evidence digest with `replayed: true`, even if the current metadata revision has
subsequently advanced. A different payload under the same key fails with
`metadata-idempotency-conflict`. A fresh key with a stale revision fails with
`metadata-revision-conflict` and does not mutate state.

All contenders serialize through the existing per-session record lock. The
record is re-read after lock acquisition, and the complete updated session
document is written by owner-private temporary file plus descriptor-relative
atomic rename. State, sessions, and exact session directories are pinned and
validated without following symlinks. The session record must be one
owner-private, single-link regular file.

## JSON Projections

Successful attach uses envelope schema
`cli.agent-session.metadata-attach.v1`. Its `data` contains:

- Exact `id`.
- `metadata` with `attachment_id`, `label`, attachment `revision`, and
  `evidence_digest`.
- The attachment's resulting `revision` and `evidence_digest` at top level.
- `replayed`, distinguishing a first commit from an exact replay.

Successful read-back uses envelope schema
`cli.agent-session.metadata-show.v1`. Its `data` is an
`agent-session.public-metadata-view.v1` object containing exact `id`, current
revision, `matching_count`, and zero or more bounded attachment projections.
When `--label` is present, `matching_count` proves whether that exact label is
present once. Comparing its attachment identity and evidence digest with the
successful attach receipt proves that the exact logical attachment remains.

Neither projection includes raw metadata values, prompts, logs, working
directories, provider resume data, runtime identity, private record
extensions, request paths, timestamps, or replay-key material.

## Stable Failures

Automation branches on `error.code`, never message text. The bounded v1 codes
include:

- `metadata-request-invalid`, `metadata-request-too-large`,
  `metadata-request-untrusted`, and `metadata-request-version-unsupported`.
- `metadata-label-invalid`, `metadata-label-forbidden`,
  `metadata-label-unsupported`,
  `metadata-value-invalid`, `metadata-value-sensitive`, and
  `metadata-value-path-forbidden`.
- `metadata-idempotency-key-invalid` and
  `metadata-idempotency-conflict`.
- `metadata-revision-conflict`, `metadata-label-conflict`, and
  `metadata-capacity-exceeded`.
- `metadata-session-record-untrusted` and `metadata-state-invalid`.
- The existing exact-session and safe-ancestor failure codes.

Touched failures include typed `retryable`, `next_action`, and bounded
`recovery` details. Diagnostics exclude raw request data, keys, credentials,
private paths, and session-record contents.
