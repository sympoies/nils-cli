# Cross-host resume proof v1

A move transfers one stopped managed Codex or Claude conversation from host A
to host B. This contract defines the evidence used by the move protocol. In
this increment, `start --resume THREAD --move-id UUID` returns
`resume-move-unavailable` before creating a session, lock, runtime, or Console
operation. Cross-host admission requires source fencing, destination admission,
and a working abort and recovery path.

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
