# codex-cli Account Select JSON Contract v1

## Purpose

This document defines `codex-cli account select`, which chooses one configured
Codex profile by a named strategy and reports a bounded capacity summary for
every candidate. It is read-only on profile secrets. Capacity comes from the
shared rate-limit cache, so a usage reporter and the selector share one fetch.

```bash
codex-cli account select \
  --strategy current-default|next-with-capacity|default-with-capacity \
  [--after NICK] [--exclude NICK]... [--format text|json]
```

Strategy values also accept the account-broker spellings `current_default`,
`next_with_capacity`, and `default_with_capacity`. JSON output always reports
the kebab-case name.

## Candidates

- Candidates are the `*.json` profiles in `CODEX_SECRET_DIR`. The nickname is
  the filename without `.json`.
- Nicknames must match `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`, the shared account
  nickname rule. Profiles with other filenames are ignored, and so are profiles
  past the first 64 in nickname order. The rule used to accept a leading `.`,
  `_`, or `-` and names longer than 64 bytes; a profile named that way is no
  longer a candidate and must be renamed to a valid nickname.
- The default profile is the one whose content, or failing that identity,
  matches the active auth file (`CODEX_AUTH_FILE`). It is the same match
  `diag rate-limits --all` uses to mark the current account.
- Candidates are ordered by nickname in byte order. This order is the only
  tie-breaker, so a result never depends on directory listing order.
- Two nicknames can map to the same rate-limit cache key, for example `a.b`
  and `a_b`. Such profiles never read or write the shared cache. They are
  always fetched, so one profile's capacity is never reported for another.

## Capacity assessment

Each candidate is classified from a rate-limit snapshot of its 5-hour window
(`label` from the provider, for example `5h`) and its weekly window
(`label=weekly`):

| `capacity` | Rule |
| --- | --- |
| `available` | Every reported window has `remaining_percent >= 1`. |
| `exhausted` | At least one reported window has `remaining_percent < 1`. |
| `unknown` | No fresh snapshot, or a snapshot with no windows. |

The threshold `thresholds.min_remaining_percent` (currently `1`) is echoed in
every success result.

Snapshots are read in this order:

1. A cache entry younger than `CODEX_RATE_LIMITS_CACHE_TTL` (default `3m`) is
   used as `source=cache`. `CODEX_RATE_LIMITS_CACHE_ALLOW_STALE` does not apply:
   a stale entry counts as a miss.
2. For `next-with-capacity` and `default-with-capacity`, each cache miss is
   fetched once, without auth refresh, from the usage endpoint. A successful
   fetch is written back to the same cache and reported as `source=network`.
   All misses are fetched concurrently in one wave, so the fetch phase takes
   about one `CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS` (default `8`, with a
   connect timeout of `CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS`, default
   `2`).
3. Anything else is `capacity=unknown`, `source=none`. That covers a failed or
   empty fetch, `current-default` (which never uses the network), and excluded
   candidates (which are never fetched).

`default-with-capacity` first reads the cache only. When that already shows
the default as `available` and not excluded, it selects the default without
any fetch, and the other candidates keep their cache-only assessment.

## Strategies

| Strategy | Selects |
| --- | --- |
| `current-default` | The default profile, whatever its capacity. |
| `next-with-capacity` | The first `available`, non-excluded profile after the origin, wrapping around. The origin itself is never selected. |
| `default-with-capacity` | The default profile when it is `available` and not excluded. Otherwise the first `available`, non-excluded profile after it. |

- The origin for `next-with-capacity` is `--after` when given, otherwise the
  default profile. With neither, the walk starts at the first nickname.
- `--after` is accepted only with `next-with-capacity`.
- `default-with-capacity` does not fail over from a default whose capacity is
  `unknown`. It fails with `default-capacity-unknown`, so a transient fetch
  failure does not move a session to another account.
- `--exclude` names a profile that must never be selected. Excluding an
  unconfigured nickname has no effect.

## Envelope

The stable schema id is `codex-cli.account.select.v1` and the stable command is
`account select`.

Success emits:

```json
{
  "schema_version": "codex-cli.account.select.v1",
  "command": "account select",
  "ok": true,
  "result": {
    "strategy": "default-with-capacity",
    "selected": "bravo",
    "default_account": "alpha",
    "origin": null,
    "thresholds": { "min_remaining_percent": 1 },
    "cache_ttl_seconds": 180,
    "candidates": [
      {
        "name": "alpha",
        "default": true,
        "excluded": false,
        "capacity": "exhausted",
        "source": "cache",
        "min_remaining_percent": 0,
        "fetched_at_epoch": 1790000000,
        "windows": [
          { "label": "5h", "remaining_percent": 0, "reset_at_epoch": 1790003600 },
          { "label": "weekly", "remaining_percent": 60, "reset_at_epoch": 1790500000 }
        ]
      },
      {
        "name": "bravo",
        "default": false,
        "excluded": false,
        "capacity": "available",
        "source": "network",
        "min_remaining_percent": 70,
        "fetched_at_epoch": 1790000100,
        "windows": [
          { "label": "5h", "remaining_percent": 80, "reset_at_epoch": 1790003700 },
          { "label": "weekly", "remaining_percent": 70, "reset_at_epoch": 1790500100 }
        ]
      }
    ]
  }
}
```

- `default_account` and `origin` are nullable. `origin` is non-null only for
  `next-with-capacity`.
- `min_remaining_percent`, `fetched_at_epoch`, and `reset_at_epoch` are omitted
  when unknown. An `unknown` candidate has an empty `windows` array.
- Each candidate carries at most two windows. The list carries at most 64
  candidates.

Text mode prints only the selected nickname and a newline on stdout. It prints
errors on stderr.

## Errors

Failures emit the shared top-level JSON error envelope with `ok=false` and no
`result`.

| `error.code` | Exit | Meaning |
| --- | --- | --- |
| `invalid-flag-combination` | 64 | `--after` with a strategy other than `next-with-capacity`. |
| `invalid-nickname` | 64 | `--after` or `--exclude` does not match the nickname grammar. |
| `unknown-origin` | 64 | `--after` names no configured profile. |
| `no-account-profiles` | 1 | No profile with a valid nickname is configured. |
| `default-account-unavailable` | 1 | No profile matches the active auth file, or it is excluded under `current-default`. |
| `default-capacity-unknown` | 1 | `default-with-capacity` could not confirm the default's capacity. |
| `no-account-with-capacity` | 1 | No eligible candidate is `available`. |

For the last three codes, `error.details` carries `strategy`,
`default_account`, `origin`, and the same `candidates` array as a success. A
caller can still show or log capacity without a second call. Clap parse errors,
such as an unknown strategy, keep the usual usage exit code of `64`.

## Privacy boundary

Output contains only nicknames, the enumerations above, integer percentages,
and epochs. It must not contain access, ID, or refresh tokens, provider account
IDs, emails, JWT claims, secret or cache paths, or raw provider responses.
Error messages are fixed strings without paths.

## Reuse

The selection core lives in `codex_cli::account::select` as a library API, so
other surfaces can assess capacity without calling the binary:

- `discover_candidates()` returns the candidate set and the default nickname.
- `assess_candidates(set, excluded, AssessMode)` returns
  `Vec<CandidateCapacity>`. It uses `CacheOnly` or `CacheThenNetwork`.
- `assess_for_strategy(set, excluded, strategy)` applies the per-strategy
  snapshot order above.
- `select(strategy, candidates, origin)` is pure and deterministic.
  `effective_origin(strategy, candidates, after)` returns the origin it uses.
- `CandidateCapacity::from_snapshot` classifies one `RateLimitSnapshot`.

## Delegation from an agent-session account broker

`agent-session` resolves Codex credentials through an external broker that
speaks the `agent-session.codex-auth-broker.v1` argv protocol. The broker
receives `select --strategy <current_default|default_with_capacity|next_with_capacity>
[--after NICK] [--exclude NICK]... --format json` and must print
`{"schema_version":"agent-session.codex-auth-broker.v1","account":"NICK"}`,
optionally with `plan`, or exit non-zero. A broker can delegate the policy to
this command instead of computing capacity itself:

1. Pass the strategy, `--after`, and every `--exclude` through unchanged. The
   underscore strategy spellings are accepted.
2. Add `--exclude NICK` for every profile in `CODEX_SECRET_DIR` that the
   broker's allowlist does not permit. Only allowlisted nicknames can then be
   selected.
3. Run `codex-cli account select ... --format json` with a closed stdin and a
   deadline, and bound the stdout it reads.
4. On exit `0` with `ok=true`, answer `result.selected` as `account`. Resolve
   `plan` from the broker's own credential lookup, because this command never
   reads credentials into its output.
5. Map any other outcome to the broker's own failure. For
   `next_with_capacity`, `no-account-with-capacity` is the expected "no
   failover target" result.

Selection and a usage reporter that runs `diag rate-limits` or
`prompt-segment` read and write the same cache. Within
`CODEX_RATE_LIMITS_CACHE_TTL` of a usage refresh, selection does no provider
fetch.

Delegation is not fully equivalent to a broker that computes capacity itself
from `diag rate-limits --all`. A broker that needs its previous guarantees keeps
these checks on its side:

- **Freshness.** A capacity strategy may select from a cache entry up to
  `CODEX_RATE_LIMITS_CACHE_TTL` old (`source=cache`). A broker that promises a
  network-confirmed account, as the `next_with_capacity` failover contract
  does, sets `CODEX_RATE_LIMITS_CACHE_TTL=1s` in the child environment. Every
  older entry is then refetched. It can also accept the result only when the
  selected candidate's `source` is `network`.
- **Unconfigured default.** When the broker excludes every profile outside
  its allowlist, `default-with-capacity` fails over from an excluded default to
  an allowlisted profile. A broker that must instead refuse when the active
  default is not allowlisted checks `result.default_account` against its
  allowlist.
- **Order.** Rotation follows nickname byte order, not the broker's allowlist
  order. A broker that depends on a custom order must keep its own policy.
