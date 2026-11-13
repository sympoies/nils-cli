# forge-cli webhook commands v1

Contract for `repo hook` and `org hook` (GitHub only). The shared engine is
`crates/forge-cli/src/ops/webhook/`; the operation catalog entries are
`repo.hook.*` and `org.hook.*` in [forge-cli-ops-v1.yaml](forge-cli-ops-v1.yaml).

## Scope

- GitHub repository webhooks (`--repo OWNER/NAME`) and organization webhooks
  (`--org ORG`).
- No delete, no enterprise-level hooks, no GitLab hooks
  (`provider_unsupported`).
- The secret value is never printed, logged, audited, or sent by `set-url` or
  `set-active`.

## Identity and token scope

Organization hooks need an organization owner token with the `admin:org_hook`
scope. A call GitHub refuses for that reason fails with
`org_hook_scope_missing`, naming the scope.

Organization calls are authorized against the organization. With an identity
policy they match only an `org` rule for it: never a repository rule, a path
rule, a default repository, or an App profile. See the
[forge identity policy](../../../nils-common/docs/specs/forge-identity-policy-v1.md).

## `hook list`

Schema `cli.forge-cli.{repo,org}.hook.list.v1`.

- Data: `provider`, `target`, and `hooks`.
- Each hook: `id`, `active`, `events`, `content_type`, `url`, and `secret`.
- `secret` is the marker `[redacted]` when a secret is configured, otherwise
  `null`. The secret value is dropped when the response is parsed.
- The first page of 100 hooks is read. The cited GitHub pages state no
  per-target hook limit, so a target with more than 100 hooks lists only 100.

## `hook set-url` and `hook set-active`

Commands:

- `hook set-url --hook ID --from URL --to URL`
- `hook set-active --hook ID (--active | --inactive)`

Schema `cli.forge-cli.{repo,org}.hook.{set-url,set-active}.v1`. Data:
`provider`, `dry_run`, `target`, `hook`, `action`, `status`, `from`, `to`, and
`observed`.

Each command reads the hook and compares the field with the expected value. For
`set-active` the expected state is the opposite of the requested one. URLs must
be distinct absolute `http(s)` URLs without whitespace or embedded credentials.

- Already the requested value: `status` is `already_set` and nothing is
  written, so a rerun after success is a clean no-op.
- Neither the expected nor the requested value: `hook_value_mismatch` (exit
  65) and nothing is written.
- Otherwise only the changed field is sent, then the hook is read back.

The write is `PATCH .../hooks/ID/config` with `url`, or `PATCH .../hooks/ID`
with `active` and no `config`. The read-back requires every other setting to be
unchanged: events, content type, `insecure_ssl`, the active flag or URL, and
whether a secret is set. Otherwise the command fails with
`hook_readback_mismatch` (exit 1); the change may have been applied. Any
failure of the write itself or of the read-back (for example a 5xx or a timeout)
is reported the same way, since the outcome is then unknown.

`--dry-run` performs the read and reports `would_update`, writing nothing.
Rollback is the same command with `--from` and `--to` swapped, or the other
state.

## `hook batch`

Command `hook batch --file FILE`, schema `cli.forge-cli.{repo,org}.hook.batch.v1`.
Omit `--repo` and `--org`: rows name their own targets.

The file is a JSON array of rows. Unknown keys are refused.

- `action` is `set-url` with string `from` and `to`, or `set-active` with
  boolean `from` and `to`.
- `target` is `OWNER/NAME` for `repo hook` and the organization for `org hook`.
- `action: "redeliver"` instead takes either a nonempty `delivery_ids` array
  of positive integers, or `status` (default `failed`), `since` and `until`,
  with the same mutually exclusive selection as `hook redeliver` below. It has no `from`
  or `to`. Each selected delivery produces a result row; an empty selection
  produces none. Mixed change and redelivery rows run in file order.

The whole file is validated before any provider call (`batch_invalid`, exit
65): malformed or duplicate rows, wrong value types, or no rows.

Each row then runs independently and all-or-nothing as above, under its own
identity scope in one process, so the verified actor is reused.

- Data: `provider`, `dry_run`, `total`, `failed`, and one entry per row in file
  order.
- Row `status`: `updated`, `would_update`, `already_set`, `failed` (with
  `error.kind` and `error.message`), or `skipped`.
- Text output is one line per row. The exit code is 1 when any row failed.
- A `hook_readback_mismatch` skips the remaining rows, since it can mean a write
  changed more than intended.

## `hook redeliver`

Command: `hook redeliver --hook ID --status failed --since TIMESTAMP --until
TIMESTAMP`, with `--repo OWNER/NAME` or `--org ORG`.
For an exact set use `hook redeliver --hook ID --delivery-id DELIVERY_A
--delivery-id DELIVERY_B` with the same target flags.

Schema `cli.forge-cli.{repo,org}.hook.redeliver.v1`. Data: `provider`, `dry_run`,
`total`, `failed`, `rows`, using the existing JSON envelope. Text emits one
line per result with target, hook, delivery id and status.

- `--status` is `failed` (default), `success`, or `all`. Success means HTTP
  200–399, including redirects; other numeric codes, including zero for a
  transport failure, count as failed. A null code is pending.
- Repeatable `--delivery-id` selects exactly the supplied positive ids across
  all delivery pages, deduplicated in provider list order. It cannot be
  combined with `--status`, `--since` or `--until`. Every requested id must
  exist on that hook before any POST; otherwise the selection fails with
  `hook_delivery_not_found` (exit 1), naming the missing id. Batch rows use
  `delivery_ids` with the same preflight per hook; other batch rows still run
  independently. Dry-run shows only those ids and their current status.
- `--since` is inclusive; `--until` is exclusive. Both are RFC 3339 timestamps
  with an offset, and the start must precede the end. Filtering is local after
  fetching all delivery pages with `gh api --paginate --slurp` (GitHub uses
  cursor links). The selected ids are frozen before any POST and deduplicated.
- Each selected delivery receives exactly one
  `POST .../hooks/ID/deliveries/DELIVERY/attempts`. A failed POST or read-back
  reports that row and continues with the remaining deliveries and batch rows.
  There is no automatic retry: a failed POST can have an uncertain outcome.
- Redelivery is asynchronous (`202 Accepted`). After each accepted POST the
  command lists deliveries again, finds a new redelivery with the original
  GUID and reads its metadata. `delivery` is the selected original id;
  `observed_delivery` is the new attempt's id. `delivery_status` is `success`,
  `failed`, or `pending`, with numeric `status_code` (null while pending).
  If no new attempt is visible, the status is pending; no completion is claimed
  and the original failure is never presented as the new attempt. No polling.
  Multiple new candidates or an unresolved earlier POST for the same GUID
  keep read-back pending because attribution to a specific POST is ambiguous.
- Row `status` is `redelivered`, `would_redeliver`, or `failed`. Failed POSTs
  and read-backs include a typed `error`; failed receiver responses retain the
  observed HTTP code. `failed` counts failed result rows; any failure exits 1.
  A listing/authorization failure in a batch has a null `delivery` and counts
  as a failed row. Pending accepted attempts exit 0 and need later confirmation.
- `--dry-run` lists selected ids and their current status, with no POST or
  delivery-detail call. Payloads, headers, free-form provider status strings,
  secrets and backend error details never appear in either output format.

Batch example (omit global `--repo` / `--org`):

```json
[{"target":"OWNER/NAME","hook":7,"action":"redeliver","status":"failed","since":"2026-01-02T00:00:00Z","until":"2026-01-03T00:00:00Z"}]
```

Exact-set batch example:

```json
[{"target":"OWNER/NAME","hook":7,"action":"redeliver","delivery_ids":[11,12]}]
```

GitHub only permits redelivery of deliveries from the
[past three days](https://docs.github.com/en/webhooks/testing-and-troubleshooting-webhooks/redelivering-webhooks).
Run recovery promptly after a URL switch; an older time window cannot restore
expired deliveries. Repository and organization webhook token permissions and
identity rules are the same as for the existing write commands.

## `org hook create`

Command: `org hook create --org ORG --url URL --events a,b --content-type
(json|form) --secret-env VAR [--active | --inactive]`.

Schema `cli.forge-cli.org.hook.create.v1`. Data: `provider`, `org`, `dry_run`,
`status` (`created` or `would_create`), `id`, `active`, `events`,
`content_type`, `url`, and `secret` (always the marker).

- The secret is read only from the named environment variable
  (`secret_env_missing` when unset or empty).
- It is sent in a private request-body file, never on a command line, and is
  never echoed.
- A hook that already delivers to the same URL is refused with
  `hook_url_exists`, so a retry cannot create a duplicate.
- `--dry-run` lists the existing hooks and creates nothing.

## GitHub documentation

- [Repository webhooks](https://docs.github.com/en/rest/repos/webhooks)
- [Organization webhooks](https://docs.github.com/en/rest/orgs/webhooks)

`PATCH .../hooks/ID/config` takes `url`, `content_type`, `secret` and
`insecure_ssl` and uses the secret "if provided". The documentation is silent on
what happens to an omitted key, so that sending only `url` keeps the secret is
undocumented: the read-back checks it and the smoke procedure below confirms it.

`PATCH .../hooks/ID` states: "If you previously had a secret set, you must
provide the same secret or set a new secret or the secret will be removed." The
active flag has no configuration-endpoint form, so `set-active` sends only
`active` (never `config`) and the read-back detects a dropped secret.

GitHub does not document that a `PATCH` without `config` leaves the secret
alone. An operator runs the
[webhook migration smoke procedure](../runbooks/webhook-migration-smoke.md) once
on a throwaway hook with a secret before a real migration.
