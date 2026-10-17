# forge-cli: Checks and review reads

Part of the [forge-cli v1 contract](forge-cli-spec-v1.md). Shared provider
parity, naming, [lock-down rules](forge-cli-spec-v1.md#lock-down-policy),
[exit codes](forge-cli-spec-v1.md#exit-code-map), and
[configuration](forge-cli-spec-v1.md#configuration) remain in the main spec.
The [operation catalog](forge-cli-ops-v1.yaml) owns backend mappings,
validation rules, and output schema versions.

## Operations

### `pr wait-checks`

- Input: `<id>` (or `--head <branch>` to resolve PR by branch),
  `--timeout <duration>` (default `30m`), `--interval <duration>`
  (default `20s`), `--required-only` (default `true`).
- Behaviour: polls the backend until every required check is in a
  terminal state (`success`, `failure`, `cancelled`, `skipped`,
  `neutral`). `--required-only=true` ignores non-required checks for
  the gating decision but still reports them in `data.checks`. On GitHub,
  when the head reports visible check rows but no required ones (a
  repository without branch-protection required checks), every visible row
  gates instead: a queued or in-progress row keeps the wait pending, and a
  failed row fails it. On
  GitHub, each gate snapshot resolves the current PR head SHA and base branch,
  reads check runs and combined status contexts for that exact SHA, and reads
  the base branch's configured required checks from branch protection and all
  active matching repository/organization rulesets. Missing required contexts
  produce pending rows even when optional checks have already finished. Checks
  configured for a specific GitHub App must match that App's id. The snapshot
  rereads PR head/base metadata before returning; a changed head or base yields
  a pending row and the waiter retries. `pr merge` uses the same fresh snapshot.
  Provider failures propagate rather than accepting a partial required set;
  malformed or incomplete configuration fails with `checks_snapshot_incomplete`.
  Ruleset-required workflows are also refused with that code: a workflow path
  cannot be proven complete from named check runs alone.
  Truncated check/status pages add a pending row, and a full rules page is
  refused because additional requirements may exist. `--allow-no-checks` does
  not bypass configured missing checks. The one-shot `pr checks` read retains
  its rollup and permission-fallback behavior. On GitLab,
  numeric MR ids use `glab mr view -F json` for the MR head pipeline and
  `glab api --hostname <host> projects/<project>/pipelines/<id>/jobs`
  for job rows; `allow_failure=true` jobs remain visible in
  `data.checks` but are not required. Branch-only GitLab snapshots
  without project context fall back to the version-pinned
  `glab ci status -b <branch>` text parser.
- Terminal states map to envelope `ok`:
  - all required `success`, with at least one check reported → `ok = true`.
  - any required `failure`/`cancelled`/`timed_out` → `ok = false`,
    exit `RUNTIME 1`, `error.kind = "checks_failed"`.
  - timeout reached with checks still running → `ok = false`, exit
    `UNAVAILABLE 69`, `error.kind = "checks_timeout"`.
  - timeout reached having never seen a required check or a visible row →
    `ok = false`, exit `DATA 65`, `error.kind = "checks_not_registered"`.
    An empty snapshot is **not** terminal (rule 8): the poll continues
    through the provider's check-registration window rather than reading
    "nothing is failing" as "everything passed". `--allow-no-checks`, or a
    repository `[checks] none = true` declaration, makes the empty set
    terminal for a repository that configures none.
  - GitHub no-CI fast fail: once the head has reported nothing for 60s, the
    wait asks `gh api repos/<owner>/<repo>/actions/workflows`. When GitHub
    reports `total_count: 0` the wait stops early with the same
    `checks_not_registered` / `DATA 65` instead of polling out its budget.
    Any status or check run (including one from an external CI app) counts
    as registered, and a failed or non-zero probe keeps the ordinary wait.
    `checks_not_registered` envelopes carry an `error.hint` naming both
    opt-outs.
- Output schema: `cli.forge-cli.pr.checks.v1`,
  `data = { state, required_count, success_count, failed:[…], pending:[…], checks:[…], duration_ms, warnings? }`.

### `pr review-threads` (read)

- The `pr review-threads list <id>` read surface emits
  `cli.forge-cli.pr.review-threads.v1`. Each thread now carries an
  `id` handle, complete ordered comment stream, and normalized diff anchor in
  addition to its state fields:
  `data.threads[] = { id, resolved, outdated, author, path, diff_side,
  line, original_line, original_start_line, start_diff_side, start_line,
  subject_type, created_at, url, body, comments }`. On GitHub `id` is the
  `reviewThreads` node id
  (`PRRT_...`) — the single handle consumed by [both thread write operations](forge-review-publication-v1.md#pr-review-threads-resolve--pr-review-threads-reply)
  (as `threadId` for resolve and `pullRequestReviewThreadId` for
  reply). On GitLab `id` is the discussion id. The field is additive;
  existing consumers are unaffected.
- Text output includes the same thread id on each thread line so terminal users
  can copy the `--thread` value without switching to JSON.
- GitHub thread and per-thread comment connections are independently paginated.
  Pagination continues until each provider connection is exhausted, without a
  fixed page-count ceiling. Paginated connections must expose a stable
  `totalCount`, and each page must make bounded progress toward it. The PR head
  and thread identity must remain stable across the complete read; partial
  GraphQL results, missing required anchor/state fields, cursor loops, count
  mismatch, or head drift fail closed as `review_snapshot_incomplete`.
- Successful JSON snapshots include
  `data.completeness = { threads: true, comments: true }`. Internal callers that
  intentionally fetch only root-comment fingerprints return `comments: false`,
  so automation can require complete reply context without inferring it from a
  thread count or page size.
- `--dry-run` emits the planned thread-list backend call without running the
  preliminary PR/MR view lookup or touching the provider network. It resolves
  the repo/project from `--repo` or the configured remote URL.

### `pr reviews` (read)

- `pr reviews <id>` emits `cli.forge-cli.pr.reviews.v1` and is GitHub-only in
  v1. GitLab and local return `provider_unsupported` (`USAGE 64`) rather than
  claiming an empty snapshot.
- The operation reads the PR's `headRefOid` and all pages of native review
  objects (100 nodes per page, with a 100-page safety limit), then
  returns `data = { provider, number, url, head_sha, current_head_reviews,
  stale_reviews, pending_reviews }`. Submitted reviews include `id`,
  `database_id`, `url`, `author`, native `state`, `commit_sha`, `submitted_at`,
  `summary`, and `summary_truncated`. Provider-valid `PENDING` reviews have no
  `submitted_at`; they are returned separately under `pending_reviews` and
  never participate in submitted-review convergence.
- `summary` is evidence only and is bounded to 4096 UTF-8 bytes. The operation
  does not parse natural-language review prose to derive a verdict. Reviews
  whose `commit_sha` differs from `head_sha` remain visible under
  `stale_reviews` but do not satisfy current-head policy.
- `--dry-run` emits the planned GraphQL call. Repository resolution uses
  `--repo owner/name` or a recognized forge remote.
- GraphQL partial errors, missing required review/page fields, unknown native
  states, cursor loops, a head change between pages, or the page safety limit
  return `review_snapshot_incomplete` (`DATA 65`). The gate never treats a
  partial native-review snapshot as empty.
