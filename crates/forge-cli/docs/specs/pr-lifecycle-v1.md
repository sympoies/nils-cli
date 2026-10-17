# PR lifecycle metadata v1

The standalone `forge-cli pr view <id> --format json` response retains
`cli.forge-cli.pr.view.v1` and all existing fields. These fields are additive:

| Field | Provider source and null semantics |
| --- | --- |
| `created_at` | GitHub `createdAt`, GitLab `created_at`; null if absent or unsupported (including Local). |
| `ready_for_review_at` | GitHub's most recent `ReadyForReviewEvent.createdAt`; null when never promoted, including a PR opened ready, or unsupported. A later conversion back to draft does not erase historical promotion. |
| `merge_queue` | Current GitHub queue entry as described below; null when not queued, no queue is configured, or the host/provider cannot expose it. |
| `merge_group` | Current queue entry's synthetic `headCommit` and its paginated check runs; null when that commit is absent, indistinguishable from the PR head, or the queue entry no longer exists. |

Timestamps are provider values. No timestamp defaults to the current time,
PR creation time, comment time, or a local observation time. An unknown stage
remains null. The existing `merged_at` retains its meaning. Text rendering and
mutation flows do not add lifecycle reads; `pr checks` continues to report PR
head checks.

## Shared queue-read contract

`ops::merge_queue::QueueRead` is the single queue object for lifecycle reads
and future queue consumers such as guarded dequeue preflight (#2469).
`ops::merge_queue::read` reads it without performing any mutation; unsupported
providers return `None` without a backend call. The GraphQL query shape is
shared with the existing merge-policy poll.

```json
{
  "queued": true,
  "position": 2,
  "enqueued_at": "2026-01-03T03:00:00Z",
  "state": "AWAITING_CHECKS",
  "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
```

`state` is provider-native, and position is passed through without converting
its indexing. Unknown entry fields are individually null. `head_sha` is the
PR head observed through `mergeQueueEntry.pullRequest.headRefOid`; it is
separate from the synthetic merge-group commit. It is a read snapshot, not an
immutable enqueue-time attestation or mutation authorization. A dequeue
consumer must revalidate the expected head and queue membership at its own
mutation boundary. Dequeue is outside this change.

## Merge-group snapshot

`merge_group` contains `head_sha`, `started_at`, `completed_at`, and `checks`.
Each check contains provider `name`, `status`, `conclusion`, `started_at`, and
`completed_at`, with absent/empty values represented as null. Pagination covers
all returned check-run pages for the queue entry's synthetic commit. Group
`started_at` is the earliest reported check start; group `completed_at` is the
latest reported completion only when the nonempty snapshot has every check
completed with a completion timestamp. Empty or still-running snapshots have
null completion. These are check timings, not commit-author or queue-entry
creation times.

The snapshot covers the current entry, not a retained history. Once the
provider removes an entry (including after merge), queue and group are null;
no historical group is guessed from the ordinary PR head or merge commit.
Callers measuring a queue stage must retain their provider snapshots while the
entry exists. Commit-status contexts are not check-run timestamps.

## Reads and failures

GitHub JSON view performs its existing PR read, then one lifecycle/queue
GraphQL read; a current synthetic group adds a paginated check-run read. The
ready connection requests only the latest ready event, so unrelated timeline
pagination cannot hide it. A missing queue schema retries the lifecycle read
without queue fields, preserving supported ready data. Backend/authentication
failures and malformed responses remain errors, rather than being represented
as a successful null snapshot. Dry-run keeps its existing primary-call plan
and performs no lifecycle reads.

Provider references: [GitHub pull-request and queue schema](https://docs.github.com/en/graphql/reference/pulls#mergequeueentry)
and [GitHub check-run API](https://docs.github.com/en/rest/checks/runs#list-check-runs-for-a-git-reference).
