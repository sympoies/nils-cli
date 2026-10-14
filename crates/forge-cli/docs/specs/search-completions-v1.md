# Completion search v1

`forge-cli search completions` reads closed issue or merged pull request facts
from GitHub through the selected `gh api graphql` backend. It performs no
mutation, credential write, scoring, label classification, or local indexing.
GitLab and Local return `provider_unsupported` before any search read.

```sh
forge-cli --format json search completions 'repo:acme/widget label:"needs review"' \
  --kind pr --since 2030-01-01T00:00:00Z --until 2030-02-01T00:00:00Z
```

## Inputs and scope

| Input | Meaning |
| --- | --- |
| `QUERY` | Provider query, preserving quoted values and separate qualifiers. |
| `--kind issue\|pr` | Closed issues or merged PRs; default `issue`. |
| `--since TIMESTAMP` | Inclusive RFC 3339 completion-window start. |
| `--until TIMESTAMP` | Exclusive RFC 3339 completion-window end, later than start. |
| `--page-size N` | Search and nested page size, 1–100; default 100. |
| `--max-pages N` | Search pages per invocation, 1–10; default 10. |
| `--max-connection-pages N` | Pages per label or closing-reference connection per invocation, 1–100; default 10. |
| `--cursor TOKEN` | Opaque continuation returned by this command. |
| `--exclude-forks` | Exclude canonical repositories with `is_fork=true`. |
| `--exclude-archived` | Exclude canonical repositories with `is_archived=true`. |

The existing search identity and authority rules apply. Without an explicit
global `--repo`, an `org:`, `user:`, or `repo:` query qualifier owns scope.
Otherwise the explicit or detected repository is added to the provider query.
`data.repo` is empty for a query-scoped read; each item's `repo` remains its
canonical `owner/name`. The backend call retains the selected authority and
the normal identity and rate-limit boundary.

The command adds `is:issue is:closed closed:…` or
`is:pr is:merged merged:…` to the query. Timestamps normalize to UTC. Search
uses an inclusive whole-second superset; canonical `closedAt` (issues) or
`mergedAt` (PRs) then enforces the exact half-open interval `[since, until)`.
`updatedAt` is exported only as a fact and never substitutes for completion.
Contradictory caller qualifiers remain provider query constraints.

`--dry-run` validates the window and continuation and emits `data.plan` for
the first required GraphQL call without reading the provider.

## Envelope and facts

The schema is `cli.forge-cli.search.completions.v1`. Normal reads return
`data.provider`, `host`, `repo`, `kind`, `query`, `effective_query`, normalized
`window { since, until }`, `item_count`, `items`, and `pagination`.
Existing `search issues`, `prs`, and `refs-to` v1 envelopes remain unchanged.
The issues/PRs backend now receives separate keywords, including quoted
qualifier values, rather than quoting an entire structured expression as one
keyword. Unfinished quotes/escapes and empty queries return `query_invalid`.

| Item field | Contract |
| --- | --- |
| `node_id`, `repo`, `kind`, `number`, `url`, `title` | Canonical identity; `kind` is `issue` or `pr`. |
| `repository` | `name_with_owner`, nullable `is_fork`, nullable `is_archived`. |
| `state`, `state_reason` | Raw provider state; nullable issue state reason, null for PRs. |
| `created_at`, `closed_at`, `merged_at`, `updated_at` | Provider timestamps; inapplicable or unavailable values are null. |
| `completed_at`, `in_window` | Issue close or PR merge timestamp and exact-window membership; unknown membership is null. |
| `author` | Nullable `{ login, kind }`; `kind` is the raw provider type such as `User` or `Bot`. An explicit deleted-author null is valid. |
| `additions`, `deletions`, `changed_files` | Nullable PR statistics; null for issues or unavailable fields, never fabricated zero. |
| `labels` | Raw label names, deduplicated without interpretation; null means unavailable, `[]` means observed empty. |
| `closing_issues_references` | PR references as `{ repo, kind: "issue", number, url }`, including other repositories; null for issues or unavailable connections. |
| `labels_pagination`, `closing_references_pagination` | Independent connection coverage described below. |
| `missing_fields` | Missing/invalid metadata field names. Missing canonical identity rejects that search page rather than fabricating an item. |

Duplicate entities within an invocation use canonical `(repo, kind, number)`
identity; references deduplicate by canonical `(repo, number)`. Label spelling
and provider author types remain unchanged. Fork/archive filters use returned
repository facts. Unknown filter facts remain visible as missing metadata;
they are never silently treated as false.

## Coverage and continuation

Partial reads still emit a successful envelope and exit zero so observed facts
remain available. Consumers must inspect coverage before declaring completion.
Window/cursor validation failures return DATA 65; unsupported providers use the
existing structured error envelope.

`data.pagination` contains:

- `status`: `complete`, `truncated`, or `partial`. Failures, missing metadata,
  and continuation fragments take precedence as `partial`.
- `complete`: this packet covers the search from its start, without a ceiling,
  outstanding pages, failed reads, missing facts, or connection fragments.
- `traversal_complete`: no remaining accessible search/nested pages or read
  failures, and no search ceiling; it does not imply that earlier packets are
  included or that metadata is available.
- `covered_from_start`, `resumed`, `start_cursor`, `next_cursor`: packet origin
  and an opaque continuation, null when no further accessible traversal exists.
- `pages_read`: search pages read in this invocation. `scanned` is the cumulative
  raw search-node count, before exact-window/repository filtering and deduplication.
- `total_count`: provider search count, null until known. `search_ceiling`
  explicitly records counts above GitHub's 1,000-result search ceiling.
- `pending_connections`: entities with unfinished or failed nested reads.
- `reasons`: `search_ceiling`, `search_pages_remaining`,
  `nested_pages_remaining`, `read_failure`, `missing_fields`,
  `nested_continuation_fragment`, or `search_continuation_fragment`.
- `failures`: `{ connection, node_id, kind }` records. Backend refusal, GraphQL
  errors, malformed pages, non-advancing cursors, and count mismatches cannot
  become complete empty data. Provider error text is not copied into facts.

Each connection coverage contains `start_cursor`, `end_cursor`, `next_cursor`,
nullable `has_next_page` and `total_count`, `observed_count`, `pages_read`,
`covered_from_start`, `complete`, `remaining_complete`, and nullable `failure`.
Counts measure observed raw edges before deduplication. `complete` requires
coverage from the connection start and agreement with its total count.
`remaining_complete` can be true for a finished suffix while `complete` stays
false. An inapplicable issue closing-reference connection has zero counts and
complete coverage, with a null reference value.

Continuations bind the selected provider/authority, effective query, kind,
exact input window strings, and fork/archive filters. Reuse the same inputs;
page bounds may change. Never interpret or construct cursor internals. Resuming
first drains pending nested connections, then continues search pages. Keep
earlier packets and assemble by canonical identity, combining connection
fragments according to their start/end cursors. A finished suffix alone is not
whole-connection evidence. Search is a live provider view, not a snapshot;
changed counts are surfaced as coverage failures where observed.

When the search ceiling is reached, `complete` and `traversal_complete` remain
false even if every accessible row was read, and `next_cursor` can be null.
Narrow the query or completion window before retrying. Missing metadata can
also leave a partial terminal packet; a null continuation is never proof of
complete facts. GitHub documents the connection ceiling in its
[GraphQL schema](https://github.com/octokit/graphql-schema/blob/master/schema.graphql).

## Illustrative response

This abbreviated neutral example shows selected fields; it is not runtime
evidence. Other documented fields remain present in actual output.

```json
{
  "schema_version": "cli.forge-cli.search.completions.v1",
  "ok": true,
  "data": {
    "provider": "github",
    "host": "github.com",
    "repo": "",
    "kind": "issue",
    "query": "repo:acme/widget",
    "window": {"since": "2030-01-01T00:00:00Z", "until": "2030-02-01T00:00:00Z"},
    "item_count": 1,
    "items": [{
      "repo": "acme/widget", "kind": "issue", "number": 7,
      "state": "CLOSED", "state_reason": "COMPLETED",
      "completed_at": "2030-01-10T12:00:00Z",
      "updated_at": "2031-01-01T00:00:00Z",
      "author": null, "labels": ["needs review"], "missing_fields": [],
      "labels_pagination": {"complete": true, "remaining_complete": true, "covered_from_start": true}
    }],
    "pagination": {
      "status": "complete", "complete": true, "traversal_complete": true,
      "covered_from_start": true, "resumed": false, "next_cursor": null,
      "search_ceiling": false, "reasons": [], "failures": []
    }
  }
}
```
