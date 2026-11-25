# Cross-host resume proof v1

A move transfers one stopped managed Codex or Claude conversation from host A
to host B. Direct local start uses `agent-session start --agent PROVIDER
--resume THREAD --move-id UUID --account ACCOUNT --cwd CHECKOUT`.
The Console transport refuses move requests. Resume checks do not acquire a
session writer or create a runtime.

## Signed records

Each record is a JSON object with exactly two strings, `payload` and
`signature`. The payload is the exact UTF-8 JSON document, at most 16 KiB. The
signature is a detached Ed25519 SSHSIG in OpenSSH armor, at most 1 KiB. Sign and
verify the payload's decoded UTF-8 bytes, including any whitespace. SHA-256
references are the lowercase hex digest of those same bytes.

The SSHSIG namespace is the record's schema identifier. The verifier uses the
expected host identity and namespace before decoding claims. Unknown versions,
duplicate fields, extra fields, and invalid field types refuse.

| Schema and namespace | Signer | Payload fields in addition to `schema_version` and `binding` |
| --- | --- | --- |
| `agent-session.resume-move-ticket.v1` | Host B | `issued_at`, `expires_at` |
| `agent-session.resume-move-stop-proof.v1` | Host A | `ticket_sha256`, `source_generation`, `account_binding_revision`, `source_checkout_sha256`, `stopped`, `issued_at`, `expires_at`, `history` |
| `agent-session.resume-move-no-writer.v1` | Host B | `stop_proof_sha256`, `verdict` with the value `aborted_no_writer` |

Every binding carries the following fields.

| Fields | Meaning |
| --- | --- |
| `move_id`, `nonce` | Unique move UUID and fresh B-generated UUID |
| `source_host`, `destination_host` | Distinct, locally configured signer identities |
| `source_session_id`, `source_incarnation` | Retained managed record ID and exact source runtime incarnation UUID |
| `provider`, `provider_session_id` | `codex` or `claude` and the provider conversation UUID |
| `account` | Shared broker account alias |
| `destination_checkout_sha256` | Digest of the checkout binding pinned by B |

Hosts use dedicated local Ed25519 signing keys and a host-local OpenSSH
allowed-signers file with an exact principal and public key for each trusted
peer. Operators provision the private keys locally and distribute only their
public keys. The verifier receives absolute local paths for `ssh-keygen` and
the allowed-signers file. It requires the SSHSIG-capable OpenSSH command and
uses the configured allowlist as the trust root.

## History and checkout binding

`history` contains `role`, `relative_path`, `bytes`, and `sha256`. The
transfer contains a single regular file with the exact original conversation
bytes and the signed metadata. Provider and role must agree.

| Provider | Role | Relative path and destination |
| --- | --- | --- |
| Codex | `codex_rollout` | `sessions/YYYY/MM/DD/rollout-…-THREAD.jsonl` under a B-owned Codex home |
| Claude | `claude_transcript` | `THREAD.jsonl` under the B-owned Claude project directory derived from B's verified checkout |

The destination uses its own broker and provider configuration and rebuilds
local indexes. Credentials, signing keys, bearer tokens, auth files, keychain
entries, environment dumps, runtime capabilities, managed session records,
locks, provider databases and WAL files stay local. A history must be
self-contained. Paginated/DB-backed histories, archived histories, and
transcripts with required external files cannot use this single-file contract.

Host A binds its canonical cwd to the provider history. Host B pins a local
checkout mapping and checks it again at admission. The source cwd digest
describes the cwd recorded in the imported history. B opens files only under
its configured local roots and launches with its verified local cwd. A's
absolute paths are never used as B filesystem paths.

## Freshness and refusals

Ticket and proof timestamps are UTC Unix seconds. Each validity interval is
positive and at most one hour. For admission, reject an expired record or a
record issued more than 60 seconds in the future. A proof may be issued up to
60 seconds before the ticket's issue time to allow for clock skew, and its
expiry must be within the ticket's expiry. The file's byte count and hash
must match the signed snapshot regardless of copied file timestamps. Expiry
does not release the source fence.

The verifier returns typed reasons.

| Code | Condition |
| --- | --- |
| `resume-move-proof-invalid` | Malformed, unauthenticated, or tampered record |
| `resume-move-proof-expired` | Invalid window, expired, or future issued |
| `resume-move-binding-mismatch` | Wrong move, nonce, or signed record reference |
| `resume-move-session-mismatch` | Wrong source record, incarnation, provider, or conversation |
| `resume-move-target-mismatch` | Wrong source or destination host |
| `resume-account-mismatch` | Account differs from the original binding |
| `resume-source-live` | Stop verdict is false |
| `resume-move-path-mismatch` | Checkout binding differs |
| `resume-move-history-stale` | Staged history differs from the snapshot |
| `resume-move-history-unsupported` | File role or placement falls outside the provider manifest |
| `resume-move-destination-writer-present` | Destination reports an existing writer |
| `resume-move-ssh-keygen-unavailable` | Configured executable is missing or lacks SSHSIG |
| `resume-move-trust-unavailable` | Configured trust file cannot be used |

Cryptographic verification and a matching snapshot establish evidence for
admission. The lifecycle owner must still serialize admission against abort,
confirm destination writer absence, and retain a terminal receipt before
recovering A. A no-writer receipt matches the exact signed stop proof even
after the proof's admission window expires. Unknown or admitted outcomes
keep A fenced.

## Read-only source plan

`agent-session resume-move plan --session ID --incarnation UUID
--destination HOST --format json` inspects the retained source and responds
with `cli.agent-session.resume-move-plan.v1`. Configure
`AGENT_SESSION_RESUME_MOVE_HOST` with the source host's signing principal.
Planning can run before the source stops and does not acquire a writer or
resolve credentials.

The plan returns the exact managed ID, incarnation and generation, provider
conversation and settled account binding, source checkout digest, and file
placement and size. The source-relative path is below the selected source
provider home. The destination-relative path follows the provider placement
table above. When the cwd belongs to a Git checkout with a configured remote,
the plan includes a digest of the canonical repository identity, the HEAD
commit, and the cwd relative to the checkout root.

The source fence uses the existing conversation guard and checks that all
source writers have stopped. Recovery requires the exact signed abort receipt
and rechecks the source snapshot before releasing the fence.

For source mutation, configure `AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN`,
`AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE`, and
`AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE` with absolute local
paths. The signing key stays in an owner-private file and the allowlist
pins the local signer and the participating peers. A stopped source export
consumes a private destination ticket and names the expected host and
incarnation explicitly. Source recovery consumes a private no-writer
receipt for the retained move ID.

## Destination reservations

The read-only `resume-move prepare --check` command takes the source plan file,
a move UUID, and the proposed local cwd. It checks the mapped checkout, account
roster, provider capability, and existing conversation writer evidence. A
different path requires matching Git repository identity, HEAD, and relative
cwd. The reservation pins the canonical local directory and its filesystem
identity. The destination rechecks it before admission.

A reserved destination returns a newly signed ticket and its private staging
home. The transfer copies only the one history file. `resume-move stage`
checks the ticket and stopped source proof against the local snapshot.
Immediately before native start, the destination rechecks the exact proof,
checkout and bytes under the same control lock used to abort. Provider
argument overrides are refused for a move. Codex recreates local indexes in
the destination provider home. Claude uses B's verified broker account and
installs the exact transcript in the local project directory after writer
admission. The move supports the short ASCII project encoding and refuses
existing transcripts with the same conversation ID. Claude sidecars such as
subagent transcripts, spilled tool results, MCP task files, and uploads are
outside the single-file contract.

Before admission the destination checks provider capability and runtime
readiness, such as the private socket directory. A deterministic failure
refuses with `resume-unsupported` and leaves the move staged, so abort still
issues a no-writer receipt. The destination persists writer admission before
taking a session writer lock.
Any ambiguous result beyond that point remains fenced. An earlier abort
records a terminal tombstone before signing the no-writer receipt. The same
move cannot be admitted after that tombstone. A lost proof transfer can supply
the original proof file to `resume-move abort` for exact receipt binding. A
missing proof cannot authorize source recovery.

## Direct move sequence

1. On A, create a read-only source plan with `resume-move plan`.
2. On B, run `resume-move prepare --plan-file FILE --move-id UUID --cwd CHECKOUT`.
   The output includes a signed destination ticket and B's staging directory.
3. Stop the source on A. Run `resume-move export --session ID --incarnation UUID
   --destination HOST --ticket-file FILE` and retain the signed stop proof.
4. Transfer the one history file to B at its manifest path in the staging
   directory. Run `resume-move stage --move-id UUID --proof-file FILE`.
5. On B, run the direct start command above. Add `--check-resume` for a
   read-only check. The checks bind the exact account, conversation, checkout,
   and signed snapshot. A successful start records the managed session ID and
   runtime incarnation with the move.
6. Before writer admission, `resume-move abort --move-id UUID` can record a
   terminal abort and return the signed no-writer receipt. If B never staged
   the proof, supply it with `--proof-file`. On A, run
   `resume-move recover --move-id UUID --receipt-file FILE` to release its fence.

Use `resume-move status --move-id UUID` on B after an interrupted start. An
admitted writer without a recorded result stays unresolved. An unresolved
move cannot authorize source recovery.

## Unresolved destination outcome

Admission persists `writer_admitted` before the destination materializes
credentials, installs a Claude transcript, or launches the session. A failure
after that point, or an interrupted start, leaves the move in
`writer_admitted`. This is intentional: the destination cannot prove that no
writer started, so it never reports a no-writer abort.

Observable behavior while a move is in this state:

- `resume-move status --move-id UUID` on B reports `writer_admitted` with no
  recorded session. `created` means the start finished and names the session.
- `start` for the same move refuses with `resume-move-outcome-unknown` and
  never launches a second writer.
- `resume-move abort` refuses with `resume-move-destination-writer-present`,
  so no no-writer receipt exists.
- On A, `resume-move recover` needs that receipt, so A stays fenced and an
  ordinary resume of the conversation on A refuses with
  `resume-move-source-fenced`. Expiry does not release the fence.

What to inspect on B:

1. Run `resume-move status` and note the state and any recorded session.
2. List the managed sessions on B and look for one for the move: the ID passed
   with `--id`, otherwise `move-` followed by the move UUID. Check whether its
   provider pane is running.
3. Check whether the staged provider home still holds the verified history.

What to do:

- If a managed session for the move exists on B and is running, the
  conversation continues there. Leave A fenced. A is the stale copy.
- If no session or pane exists, B has probably not started a writer, but the
  tooling cannot prove it. Do not edit or delete the move store, the fence
  ledger, or the lock files by hand. Keep A fenced and report the move ID.
- Release of A is supported only through a signed no-writer receipt. There is
  no force option. Automatic settlement of a move whose post-admission failure
  provably started no writer is tracked as follow-up #4042. Until it lands, an
  unresolved move keeps A fenced.
