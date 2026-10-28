# Claude credential writer receipts v1

`claude-cli auth publish --snapshot <file> --into <accounts-dir> --keychain off --format json`
publishes one admitted access-only snapshot into its named account directory.
It shares the account writer with `auth remote pull --all`. It does not contact
an authority/provider, refresh, change authority profiles, select a default,
write `.current`, prune other accounts, or advertise a consumer capability.

## Input and admission

The snapshot is a single JSON profile payload with `profile`, `claudeAiOauth`,
`oauthAccount`, the existing validated `credential_generation` annotation, and
a sibling `publication_attempt_id`. The latter is a canonical lowercase UUIDv4
supplied by the caller for this publication attempt. It is not part of the
authority's immutable generation annotation. A snapshot file must be a regular,
non-symlink file owned by the effective user with mode `0600`, at most 1 MiB.
Credential values are read from the file, never command arguments.

The generation contract is unchanged: `nils.auth-generation.v1`, provider
`claude`, account matching `profile`, exact authority ID/epoch/generation,
snapshot ID, timestamps, and authentication observation. Invalid context,
missing context/attempt on `auth publish`, invalid attempts, missing access
credentials, and missing account identity fail before creating the account
store. The writer drops refresh tokens and unknown OAuth fields through the
existing access-only projection. It cannot independently authenticate a caller's
snapshot-to-generation binding; the consumer must admit that exact immutable
payload and persist intent before invoking it.

An SSH `--all` profile may carry the same sibling attempt to request a receipt.
A supplied attempt without valid generation is rejected before any account
write. All entries are validated before any profile is written. Existing
payloads without an attempt retain existing behavior and produce no receipt.
Single active-login pulls retain generation transport context only; they do
not produce account-store receipts. A receipt-required consumer uses
`auth publish` and must reject missing or mismatched evidence.

## Receipt schema

The local JSON success `result.writer_receipt` and the additive remote
`result.profiles[].writer_receipt` use this exact shape. The same object is
persisted as `.claude-cli-writer-receipt.json` in the owned account directory,
with mode `0600`:

```json
{
  "schema_version": "nils.claude-writer-receipt.v1",
  "credential_generation": {
    "schema_version": "nils.auth-generation.v1",
    "provider": "claude",
    "account": "alpha",
    "version": {
      "authority_id": "11111111-1111-4111-8111-111111111111",
      "authority_epoch": "22222222-2222-4222-8222-222222222222",
      "generation": "1"
    },
    "snapshot_id": "33333333-3333-4333-8333-333333333333",
    "authority_committed_at": "2030-01-01T00:00:00.000Z",
    "issued_at": null,
    "expires_at": null,
    "auth": {
      "revision": "1", "state": "unknown", "reason": "unvalidated",
      "source": "unknown", "observed_at": null
    }
  },
  "publication_attempt_id": "44444444-4444-4444-8444-444444444444",
  "state": "completed",
  "stores": {
    "account_credentials": {"state": "verified"},
    "account_config": {"state": "verified"}
  }
}
```

`credential_generation` is unchanged from the existing generation
contract. It includes the account nickname,
not provider account/organization identifiers. No credentials, credential-derived
digests, account identity contents, private paths, or raw errors are included.
The surrounding existing remote envelope retains its existing fields; consumers
must project the receipt alone when publishing token-free evidence.

Receipt `state` is `in_progress` or `completed`. Each store state is `pending`,
`verified`, or `failed`. Unknown versions, malformed evidence, missing evidence,
and inaccessible evidence are unavailable and must fail closed. A read or write
failure can leave `in_progress` with a verified first store and pending/failed
second store. It never establishes completion. Failure while recording evidence
is `writer-receipt-unavailable`; mismatched/unreadable store readback is
`writer-readback-failed`. These errors omit store contents and secret values.

## Ordering, durability, and recovery

The caller holds the existing accounts-directory lock throughout account
publication. Before marker or store mutation, the writer atomically replaces
prior evidence with an `in_progress` receipt containing the new exact context
and attempt and both stores pending, and syncs its directory. Failure to fence
prevents store mutation. It writes access-only `.credentials.json`, reads back
`claudeAiOauth` for exact equality to the admitted access-only projection, syncs
the file/directory, then durably records that store verified. It similarly writes
or preserves `.claude.json` and reads back `oauthAccount` for exact equality.
Other existing credential/config entries are preserved.

The existing Keychain projection runs with its existing off/auto/required
semantics. Both file stores are rechecked after projection, and completion is
persisted last, with directory-sync errors propagated. Keychain status is
returned separately: the receipt proves only the two named file stores.
A required Keychain failure leaves the attempt incomplete; automatic Keychain
unavailability retains its existing behavior. Directory preparation and link
checks belong to the consumer's completion protocol, not these store receipts.

A saved receipt records a historical writer observation, never present store
truth, provider validity, runtime adoption, or a capability. After interruption
or restart, re-run publication from the retained admitted snapshot under the
accounts lock. Never reuse pre-crash verified statuses: every invocation begins
pending and revalidates both stores, even with the same attempt and unchanged
config. Consumers compare the entire exact generation object and attempt,
require both verified plus completed, and finish their own durable intent and
completion-last protocol. They own trusted authority binding, high-water/replay
fences, directory/link checks, and freshness against nonparticipating writers.
There is deliberately no standalone receipt-verification command.

## Writer and cleanup inventory

| Path | Receipt consequence |
| --- | --- |
| Receipt-bearing remote all-profile import and local `auth publish` | Shared writer; fresh fence, per-store readback, completion last under accounts lock. |
| Receipt-free remote all-profile import | Removes/syncs old receipt before writing, no new receipt. |
| Refresh/auto-refresh account projection | Uses receipt-free account writer under existing accounts lock; invalidates before writing, no receipt. Refresh itself is outside this feature. |
| Active projection from profile use/save/refresh or single remote pull | Joins accounts lock when targeting a managed account; invalidates evidence before store mutation, no receipt. |
| Delegated `auth login`/`auth logout` | Fences active-store evidence and joins a managed account's lock before launching upstream, even if upstream later fails. |
| Owned-account prune | Removes/syncs evidence before recursive deletion; unowned directories remain untouched. |
| Profile save/remove/quarantine/current and accounts `.current` updates | Do not mutate the two account stores; no receipt is manufactured. |
| Temporary refresh work directory | Isolated scratch stores, no account receipt. |
| Arbitrary upstream runtime, external file edits/deletion, or older binaries | Nonparticipating; stored evidence is not authoritative. Consumer capability must stay unavailable unless its fresh publication/readback and external-writer exclusion protocol is established. |

No broker capability enablement, real-provider switch/resume acceptance,
refresh, rotation, automatic failover, or release is included in this contract.
