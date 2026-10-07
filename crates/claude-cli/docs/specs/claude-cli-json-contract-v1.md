# claude-cli JSON Contract v1

## Purpose

This specification extends
`docs/specs/cli-service-json-contract-guideline-v1.md` for:

- `claude-cli usage --format json`
- `claude-cli prompt-segment status --format json`
- `claude-cli auth status --format json`
- the `claude-cli auth` profile commands listed under
  [Auth profiles](#auth-profiles)
- `claude-cli agent doctor --format json`
- the `limit_resets` field of `claude-cli diag rate-limits --format json`

`claude-cli auth reset-rate-limits` has its own contract,
[`claude-cli-auth-reset-rate-limits-json-contract-v1.md`](claude-cli-auth-reset-rate-limits-json-contract-v1.md).

Text remains the default. JSON is opt-in and is emitted to stdout as one
versioned envelope.

## Schemas

| Command | `schema_version` | Payload |
| --- | --- | --- |
| `usage` | `claude-cli.usage.v1` | `result` or `error` |
| `prompt-segment status` | `claude-cli.prompt-segment.v1` | `result` |
| `auth status` | `claude-cli.auth.v1` | `result` or `error` |
| `auth save`, `auth use`, `auth remove`, `auth current`, `auth refresh`, `auth auto-refresh`, `auth remote pull` | `claude-cli.auth.v1` | `result` or `error` |
| `agent doctor` | `claude-cli.agent.doctor.v1` | `result` |
| `diag rate-limits` | `claude-cli.diag.rate-limits.v1` | `result`, `results`, or `error` |
| `auth reset-rate-limits` | `claude-cli.auth.reset-rate-limits.v1` | `result` or `error` |

Every envelope contains `schema_version`, `command`, and `ok`. Additive fields
are compatible within v1. Renaming, removing, or changing the meaning of
stable fields requires a new schema version.

## Usage

Stable result fields are `provider`, `source`, `stale`, `windows`, and optional
`reason_code`. `provider` is `claude`; `source` is `oauth`, `cli`, `cache`, or
`none`.

Each window contains `key`, `label`, `window_minutes`, `used_percent`,
`remaining_percent`, and optional `resets_at` and `resets_at_epoch`.
Informational result fields are `cache_file`, `updated_at`, `plan`, and `note`.

Consumers must not authorize a state transition from stale windows. An empty
`windows` array is a valid unavailable result, not unlimited quota.

Stable `reason_code` values:

- `auth_required`
- `auth_expired`
- `billing_past_due`
- `subscription_inactive`
- `organization_disabled`
- `permission_denied`
- `rate_limited`
- `service_unavailable`
- `timeout`
- `unknown`

### Usage error envelopes

`usage` normally succeeds, including when no window is available. It emits an
error envelope only for an operator-input or cache-maintenance failure:

| `error.code` | Exit | Cause |
| --- | --- | --- |
| `invalid-flag-combination` | `64` | `--clear-cache` combined with `--source cache` |
| `cache-clear-failed` | `1` | `--clear-cache` could not resolve or remove `usage.json` |

`--clear-cache` removes the resolved `<cache dir>/usage.json` and its
`usage.refresh.at` throttle stamp. It never removes the cache directory itself,
nor the `usage.refresh.lock` / `usage.refresh.spawn.lock` files that a running
background refresh may hold. The throttle stamp is cleared so the next
background refresh is not suppressed while no cache remains to render. A
relative `CLAUDE_PROMPT_SEGMENT_CACHE_DIR` resolves against the working
directory, matching how the cache is read and written; only a file named
`usage.json` is ever removed.

### Usage debug output

`--debug` writes one bounded line per attempted source to **stderr**:

```text
claude-cli usage: debug: source=oauth outcome=unavailable reason=auth_required elapsed_ms=12
```

`source` is `oauth`, `cli`, `transcript`, or `cache`; `outcome` is `available`
or `unavailable`; `reason` is a stable `reason_code` when one was classified.
`outcome` means the same thing for every source: `available` is reported only
when that source produced usage. The `transcript` probe classifies a prior
failure and never yields usage, so it always reports `unavailable` and carries
its classification in `reason`.
Debug output is not part of this contract, is not emitted on stdout, and never
changes the stdout envelope: a consumer parses exactly one versioned document
with or without `--debug`.

## Prompt-segment status

Stable result fields are `authenticated`, `cache_exists`, `cache_stale`,
`would_render`, and `reason`. `auth_source` and `cache_file` are informational.
`authenticated` reports prompt-segment OAuth availability, not general Claude
Code login state.

## Auth status

Stable result fields are `logged_in` and optional `auth_method`,
`api_provider`, and `subscription_type`.

The command calls `claude auth status --json` and constructs a new allowlisted
result. It drops email, organization identifiers and names, tokens, credential
paths, and unknown upstream fields.

Authenticated status exits `0`; unauthenticated status exits `1`. Either may
produce `ok: true` when the wrapper successfully inspected valid upstream JSON.

Command-level failures use `ok: false` with:

- `launch-failed`
- `upstream-timeout`
- `output-too-large`
- `invalid-upstream-output`
- `invalid-upstream-shape`
- `unexpected-upstream-status`
- `inconsistent-upstream-status`

The wrapper accepts only a top-level object with boolean `loggedIn`. Exit `0`
must pair with `loggedIn: true`; exit `1` must pair with `loggedIn: false`.
Malformed JSON, malformed shapes, and JSON/exit disagreement are data errors.
Other upstream exit values and the three-second child deadline are runtime
errors. A non-`0`/`1` exit is classified before parsing stdout, so malformed
diagnostic output cannot hide a runtime failure. Captured stdout and stderr
share one aggregate limit while the child is still running.

## Auth profiles

Profiles are stored as `CLAUDE_SECRET_DIR/<name>.json`, where the name follows
the shared account nickname rule `[A-Za-z0-9][A-Za-z0-9._-]{0,63}` (a
`name.json` target is accepted); anything else is `invalid-profile-name`
(exit `64`). The current default is the single nickname line in
`CLAUDE_SECRET_DIR/current`.

The nickname rule became stricter in the account alignment change: it used to
accept a leading `.`, `_`, or `-` and names longer than 64 bytes. A stored
profile whose name no longer fits is left out of `auth current`'s `profiles`
list, and naming it in a command is `invalid-profile-name`. A `current` file that records such a name
makes `auth current` fail. Rename the profile file to a valid nickname, then
run `auth use <name>` again if it was the current default.

Every command below takes `--format json` and
emits the `command` shown with these stable result fields:

| Command | `result` fields |
| --- | --- |
| `auth save [--yes] <name>` | `profile`, `account_uuid`, `replaced` |
| `auth use <name\|name.json\|email>` | `target`, `profile`, `account_uuid`, `credentials_file`, `config_updated`, `keychain` |
| `auth remove [--yes] <name>` | `profile`, `removed` |
| `auth current` | `matched`, `profile`, `account_uuid`, `organization_uuid`, `expires_at`, `profiles` |
| `auth refresh <name>...`, `auth auto-refresh` | `refreshed`, `skipped`, `failed[{profile, code, message}]`, optional `projected` and `projection_failed` |
| `auth remote pull --ssh <host> (--name <name> \| --current) --access-only --write-active` | `ssh`, `profile`, `account_uuid`, `expires_at`, `credentials_file`, `config_updated`, `keychain`, `has_refresh_token` |
| `auth remote pull --ssh <host> --all --into <dir> --access-only` | `ssh`, `into`, `current`, `profiles[{name, config_dir, written, keychain, expires_at, has_refresh_token, error?}]`, `pruned` |

`auth remove` refuses the current default with `profile-is-current-default`;
switch the default first. `auth use` reports an ambiguous target with
`ambiguous-profile` and `auth current` without a recorded default with
`matched: false`; both exit `2`. `auth remote export` prints the transport
payload rather than an envelope.

`auth current` and `diag rate-limits --all` agree on the current default: both
read `CLAUDE_SECRET_DIR/current`. `auth remote pull --all --into <dir>` also
records the authority's current default in `<dir>/.current` (one nickname line,
owner-only, replaced atomically under the accounts lock), the file the host
account broker reads, and removes a stale `.current` when the authority reports
none.

## Diag rate-limits limit resets

`diag rate-limits` emits the shared `diag rate-limits` result shape that
`codex-cli diag rate-limits` also emits. Each Claude result read from the
network additionally carries `limit_resets`, the normalized status of the two
Claude limit-reset programs. Results from `--cached` or a cache fallback, and
failed results, omit it. A network body without either program reports both
as `null`, so absence always means the value was not read.

Live `--async --format json` collection normally falls back to stale cached
windows after a profile request fails. A machine consumer that needs each
profile's live failure and reason can set
`CLAUDE_RATE_LIMITS_ASYNC_JSON_NO_CACHE_FALLBACK=1`; async JSON then keeps that
profile's failed result instead of replacing it with cached windows. This does
not change `--cached` reads or text output.

Async JSON also includes the active Claude Code login when its access token is
not already represented by a saved profile. Its result name is `active` unless
that nickname is already used by a saved profile, in which case an unused
`active-login` name is used. It can therefore return the active login even
when the saved-profile directory is absent or empty. Without a readable active
login, missing or empty profile storage remains a discovery error.

```json
{
  "juniper_tide": {
    "available": false,
    "eligible": false,
    "ineligible_reason": "not_at_wall",
    "arm": null,
    "resets_per_week": 1,
    "next_available_at": null,
    "weekly_resets_at": null
  },
  "cedar_ember": {
    "available": true,
    "eligible": true,
    "ineligible_reason": null,
    "at_limit": true,
    "exhausted": ["five_hour"],
    "next_grant_id": "grant_a",
    "grants": [
      {
        "id": "grant_a",
        "label": "Welcome reset",
        "resets_left": 2,
        "resets_total": 2,
        "starts_at": 1788220800,
        "ends_at": 1791763200,
        "clears": ["five_hour", "seven_day"],
        "paused": false,
        "usable_now": true,
        "use_requires_limit": true
      }
    ],
    "cooldown_until": null,
    "weekly_resets_at": 1791334800
  }
}
```

- A program is `null` when the upstream block is missing, `null`, or not an
  object. A malformed block never fails the usage result.
- `juniper_tide.available` is `eligible && available && arm != "control"`.
- `cedar_ember.grants` keeps at most 16 grants. A grant whose `id` does not
  match `^[a-z0-9_-]{1,40}$` or whose `resets_left` is not a non-negative
  integer is dropped. `next_grant_id` is kept only when it names a kept grant.
  `available` is `eligible` and that grant has `resets_left > 0` and is not
  paused. `use_requires_limit` defaults to `true`.
- `ineligible_reason` and `arm` are `null` or a bounded token
  (`^[a-z0-9_]{1,40}$`); any other string becomes `unknown`. `exhausted` and
  `clears` keep only bounded tokens. `label` is `null` unless it is at most 80
  characters without control characters.
- Every timestamp is `null` or epoch seconds converted from the upstream
  ISO-8601 string. Upstream `event_props`, `percent_used`, and `blocking` are
  dropped.
- Grant ids are host-local identifiers. A consumer that forwards
  `limit_resets` off the host must drop `next_grant_id` and each grant `id`.

## Agent doctor

Stable result fields are `ready`, `commit_profile`,
`configured_commit_profile`, `upstream_doctor`, `upstream_doctor_status`,
`dependencies`, and `flags`. `commit_profile` covers the fixed safe commit
flags. `configured_commit_profile` additionally requires optional model and
effort flags when the effective wrapper configuration selects them; `ready`
uses this configured profile.

`dependencies` contains booleans for `claude`, `git`, `semantic_commit`, and
`semantic_commit_compatible`. Presence and compatibility remain separate so an
older or unrelated executable cannot make `ready` true.
`flags` maps each required upstream option to a boolean. Stable
`upstream_doctor_status` values are:

- `ready`
- `failed`
- `timeout`
- `output-too-large`
- `launch-failed`

The command never invokes a model. It runs `claude --help` and
`claude doctor` with bounded stdout/stderr capture and emits none of the
captured text. `ok: true` means diagnosis completed even when `ready` is
false. Ready exits `0`; unavailable exits `1`.

## Examples

```json
{"schema_version":"claude-cli.usage.v1","command":"usage","ok":true,"result":{"provider":"claude","source":"cache","stale":true,"windows":[{"key":"5h","label":"5h","window_minutes":300,"used_percent":25.0,"remaining_percent":75.0}]}}
```

```json
{"schema_version":"claude-cli.auth.v1","command":"auth status","ok":true,"result":{"logged_in":true,"auth_method":"claude.ai","api_provider":"firstParty","subscription_type":"team"}}
```

```json
{"schema_version":"claude-cli.agent.doctor.v1","command":"agent doctor","ok":true,"result":{"ready":true,"commit_profile":true,"configured_commit_profile":true,"upstream_doctor":true,"upstream_doctor_status":"ready","dependencies":{"claude":true,"git":true,"semantic_commit":true,"semantic_commit_compatible":true},"flags":{"--json-schema":true,"--safe-mode":true}}}
```

## Sensitive-data rules

- Never emit tokens, API keys, raw credential JSON, authorization headers,
  upstream error bodies, or terminal transcripts.
- Auth status additionally excludes personal and organization identity.
- `diag rate-limits` and `auth reset-rate-limits` never emit organization
  uuids or upstream `event_props`.
- Agent doctor excludes upstream diagnostic stdout/stderr, settings paths,
  environment values, and model output because it does not make a model call.
- Usage `--debug` emits only source classifications and elapsed milliseconds:
  no provider bodies, terminal transcripts, credentials, or absolute paths.
- Tests seed recognizable secret markers and assert that stdout and stderr omit
  them.
