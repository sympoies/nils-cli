# Local Provider Contract v1

`forge-cli --provider local` uses a file-backed JSON store for deterministic
issue lifecycle tests and seeded pull-request reads. The source of truth is
`src/local/store.rs`; `tests/integration/local_ops.rs` exercises the public
CLI contract. Local PR creation, review, and merge are unsupported.

## Store locator and layout

`--store-root PATH` wins over `FORGE_CLI_LOCAL_STORE`; absence is an error.
`--repo local:NAME` selects a slug, stripping `local:`. One root holds one
repository:

```text
<store-root>/
  repo.json
  issues/<number>.json
  prs/<number>.json
```

`repo.json` contains `slug`, `provider: "local"`, monotonic `next_issue` and
`next_pr` counters, and a `clock` counter. `forge-cli` creates it and owns its
updates. The clock produces deterministic timestamps starting at
`2026-01-01T00:00:00Z`; no wall clock is consulted.

## Issue records

`forge-cli` owns `issues/<number>.json`. Each record contains `number`,
`title`, `body`, `labels`, `state` (`open` or `closed`), `close_reason`
(currently `null` for Local issue closes), and `comments`. A comment contains
`id`, `body`, `author`, `created_at`, and `url`. Comment URLs use
`local://<slug>/issues/<number>#comment-<id>`.

Issue create, view, list, edit, comment, and close operate on these records.
List applies all requested labels to open issues; numbering and timestamps are
monotonic within the store.

## Seeded pull-request records

Tests may write `prs/<number>.json` directly. `forge-cli` reads these records
through `pr view`, `pr checks`, and `pr comments`, but does not mutate them.
A record contains `number`, `state` (`OPEN`, `CLOSED`, or `MERGED`), `merged`,
`merge_sha`, `checks`, `required_state`, `required_count`,
`non_required_failures`, and `comments`. Each comment contains `body`,
`html_url`, and optionally `author` and `created_at`. The latter fields and
the check rollups have serde defaults, so older seed records remain readable.

Seeded state is test evidence, not a simulated merge. Consumers must not infer
that the local provider performed a PR mutation.
