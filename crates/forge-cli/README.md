# forge-cli

Provider-neutral CLI for remote forge operations (personal inbox discovery,
PR/MR lifecycle, Issue lifecycle, CI wait, and repository label catalog
maintenance). Two backends ship together: GitHub (wraps `gh`) and GitLab
(wraps `glab`). Adopts `cli-output-contract-v1` from day one.

## Read first

- Docs index: [docs/README.md](docs/README.md)
- Contract: [docs/specs/forge-cli-spec-v1.md](docs/specs/forge-cli-spec-v1.md)
- Op catalog: [docs/specs/forge-cli-ops-v1.yaml](docs/specs/forge-cli-ops-v1.yaml)
- Workspace envelope contract:
  [`/docs/specs/cli-output-contract-v1.md`](../../docs/specs/cli-output-contract-v1.md)

## Quick start

```sh
cargo run -p nils-forge-cli -- --help
cargo run -p nils-forge-cli -- inbox status --format json
cargo run -p nils-forge-cli -- auth status --format json
cargo run -p nils-forge-cli -- completion zsh
cargo run -p nils-forge-cli -- label audit --catalog labels.yaml --format json
cargo run -p nils-forge-cli -- pr deliver --kind feature --dry-run --format json
cargo run -p nils-forge-cli -- pr review 123 --decision comments-only --comment-file review.md --mirror-issue --issue 456 --format json
cargo run -p nils-forge-cli -- pr review validate --comment-file review.md --thread-file review-threads.json --format json
cargo run -p nils-forge-cli -- pr review validate --specialist-report --comment-file review.md --thread-file review-threads.json --format json
cargo run -p nils-forge-cli -- pr review validate 123 --check-diff --comment-file review.md --thread-file review-threads.json --format json
cargo run -p nils-forge-cli -- pr review 123 --decision comments-only --submit-review --expected-head <sha> --comment-file review.md --thread-file review-threads.json --format json
cargo run -p nils-forge-cli -- pr review 123 --decision approve --metadata-only --native-review-url <review-url> --native-review-author <app-login> --issue 456 --mirror-issue --format json
cargo run -p nils-forge-cli -- pr reviews 123 --format json
cargo run -p nils-forge-cli -- pr pending-review inspect 123 --review PRR_pending --format json
cargo run -p nils-forge-cli -- pr pending-review resume-submit 123 --review PRR_pending --review-run-id <digest> --expected-head <sha> --expected-commit <sha> --expected-snapshot <digest> --decision comments-only --format json
cargo run -p nils-forge-cli -- pr pending-review delete 123 --review PRR_pending --expected-head <sha> --expected-commit <sha> --expected-body-file review.md --confirm-abandoned --dry-run --format json
cargo run -p nils-forge-cli -- pr merge 123 --expected-head <reviewed-sha> --review-convergence --format json
cargo run -p nils-forge-cli -- repo freeze status --format json
cargo run -p nils-forge-cli -- issue tracker lint 123 --check-state --format json
cargo run -p nils-forge-cli -- issue tracker graph 123 --write --format json
cargo run -p nils-forge-cli -- issue tracker tick 123 --item S2 --pr owner/repo#45 --comment-file note.md --format json
cargo run -p nils-forge-cli -- repo push-default --expected-base <sha> --reason-file reason.md --dry-run --format json
cargo run -p nils-forge-cli -- repo push-default --default-branch-receipt receipt.json --expected-base <sha> --reason-file reason.md --dry-run --format json
```

Repository-scoped commands derive their target from the selected Git remote.
Use `--remote <name>` to select a remote other than `origin`. For the local
provider, `--store-root <path>` overrides `FORGE_CLI_LOCAL_STORE`; it does not
affect GitHub or GitLab operations.

## Empty repository bootstrap

`repo bootstrap` creates one signed zero-parent commit and publishes it as the
first branch of an empty repository. It supports a new GitHub user or
organization repository, an explicitly selected existing empty GitHub
repository, and the existing private Forgejo creation flow. GitLab is not
supported. Supply `--provider`, `--repo owner/name`, `--owner-kind`, a safe
`--default-branch`, one or more regular repository-root `--file` inputs, a
Semantic Commit `--message`, and a regular `--reason-file` recording the
operator's authorization. The default visibility is private; public GitHub
repositories require `--visibility public`. Forgejo remains private only.

```sh
forge-cli --provider github --repo OWNER/REPO --dry-run repo bootstrap \
  --owner-kind org --visibility public --default-branch main \
  --file README.md --message 'chore: initialize repository' \
  --reason-file authorization.txt
forge-cli --provider github --repo OWNER/REPO repo bootstrap \
  --owner-kind org --visibility public --existing-empty --default-branch main \
  --file README.md --message 'chore: initialize repository' \
  --reason-file authorization.txt
```

Omit `--existing-empty` to create a new empty repository; include it only when
adopting a repository that already exists and has no refs. The CLI records an
exact-input receipt before creation or push, verifies the local signature and
zero-parent ancestry, reads back the exact remote branch and provider signature,
reads GitHub API status from the HTTP response header,
and never retries an ambiguous first push. Receipts use schema v2; prior
Forgejo v1 receipts remain readable. If interrupted, inspect the receipt
reported in the error and repeat the same command with `--resume`. If an
attempted push has no remote branch, resume stops for manual reconciliation. A completed
matching receipt is idempotent. Nested paths belong in a normal managed
worktree and PR after the initial branch exists.

`repo push-default` is a narrow, policy-driven exception to PR delivery. It
requires a clean non-default checkout whose `HEAD` is exactly one locally
verified signed commit ahead of `--expected-base`; proves fast-forward ancestry;
uses an exact-old-object lease as a compare-and-swap; and verifies the remote
SHA afterward. The selected remote must expose exactly one push URL, and that
actual destination must match the provider repository; all remote reads and the
push are pinned to that URL. HTTP(S) userinfo and any second-stage Git URL
rewrite are rejected, including empty rewrite prefixes that match every URL.
Release builds fix the Git executable, timeout, and
capture cap; provider metadata and every Git subprocess are bounded. The
command exposes no caller-controlled force mode. Callers remain responsible for
obtaining explicit user authorization and recording it in a regular
`--reason-file`.

`--default-branch-receipt` adopts a prior governed default-branch commit as the
only checked-out-default exception. It strictly parses the final receipt and
revalidates the repository fingerprint, branch, exact head/parent/tree,
signature, one-commit ancestry, live remote base, destination, compare-and-swap,
and read-back. Receipt creation is not push authorization; provider delivery is
a separate explicit action. Adoption is eligible only for an
`aligned` to `ahead-by-one` receipt. Preview output and removed receipt schemas
are rejected before any provider or Git delivery operation.

Version 1.25.11 is the coordinated local cutover boundary: it accepts only
`--default-branch-receipt` and the matching final receipt schema. Version
1.25.10 and earlier use `--local-default-receipt`. The removed option and
receipt are intentionally not aliases, so `forge-cli` must be deployed with
the matching 1.25.11 `semantic-commit` before any governed receipt is adopted.
This source boundary does not itself publish or authorize a release.

`--thread-file` is for actionable findings only: max 50 threads, 16 KiB body
each. Use `pr review validate` for local schema/privacy checks, and add
`--check-diff` with a PR id when you want GitHub changed-file/line validation
before posting. Put non-blocking notes in the review body.

Add `--specialist-report` when the body must satisfy the canonical specialist
Review Report marker, fields, verdict vocabulary, and findings table. This is
opt-in so generic review comments remain valid. When an owner App has already
published the authoritative native review, use `--metadata-only` with that
same-PR `--native-review-url` and its expected `--native-review-author` App
login; forge-cli reads the review back and verifies its URL, decision state,
and author before posting a concise personal metadata breadcrumb and, when
requested, an issue mirror. Metadata-only mode rejects bodies, threads, native
submission, or a mismatched provider review before mutation.

`forge-cli` does NOT introduce a `--json` boolean flag. Use
`--format text|json` exclusively.

Native review convergence is compatibility-preserving and off by default.
Enable it per invocation with `--review-convergence`, per repository in
`.forge-cli.toml`, or in the user-global config. The first `observed` bot mode
never waits for a bot that has not submitted a review. Once relevant
current-head review activity exists, it waits for the configured quiet period,
reports bounded native review summaries, and blocks native
`CHANGES_REQUESTED`. The complete paginated review snapshot is read again
immediately before merge; partial provider data (including a review without a
commit OID) or late review activity fails
closed, and the initial non-empty provider head is bound through the final
merge compare-and-swap. GitHub is the only supported provider in v1; enabled
GitLab dry-runs fail with the same `provider_unsupported` result as live runs.
Merge and deliver dry-run envelopes expose the resolved policy under
`data.review_convergence`. Existing unresolved-thread enforcement remains an
independent merge gate. See the contract for config precedence, duration
bounds, and the JSON snapshot.

Provider-valid pending reviews are listed separately under
`pr reviews data.pending_reviews`; they are not submitted review activity.
`pr pending-review inspect` reads the exact draft plus every inline-comment page
and returns a stable snapshot digest and `receipt-bound` or `unmarked`
provenance. Use `resume-submit` for an exact receipt-bound transaction; it adds
no duplicate content and also succeeds idempotently when submission completed
but its response was lost. Use guarded `submit --confirm-unmarked-submit` only
after inspecting an unmarked draft. `discard` is destructive and needs a
second `--confirm-inline-content-loss` when the snapshot contains inline
comments. None of these recovery paths deletes and recreates inline content.

The older `pending-review delete` remains a body-only compatibility surface for
a confirmed abandoned draft. It verifies exact head, commit, body, PR
membership, `PENDING` state, `viewerDidAuthor`, and `viewerCanDelete`; any inline
comment fails closed. It works for GitHub App installation actors without the
user-only `GET /user` endpoint. GitHub provides no content CAS for deletion, so
a small final-read-to-mutation race remains.

Direct `pr merge` callers can pass `--expected-head <sha>` and
`--expected-base <branch>` to bind the merge to the head and target they
reviewed. Provider drift then fails before the merge mutation; `pr deliver`
binds both values internally. A requested `--base` is exact throughout lookup,
adoption, create read-back, ready, and merge; `--allow-non-default-base` only
authorizes that named target and never widens it to another non-default branch.

On GitHub, `pr merge` also honors a repository-wide merge freeze. The freeze
record is an open issue labelled `merge-freeze`, managed with
`repo freeze start --reason <text> [--until <time>]`, `repo freeze end
[--issue <n>]`, and `repo freeze status`. While a freeze is open the merge fails
closed with `merge_freeze_active`. The freeze holder can still merge by naming
every active freeze with `--allow-merge-freeze <issue>` and giving a
`--allow-merge-freeze-reason`, which is recorded as `merge_freeze_override`.
The record lives on the provider, so it works on private repositories that
cannot use branch protection or a merge queue. The freeze is read again
immediately before the merge or enqueue, so a freeze started while the gates
ran still blocks. `pr deliver` has no freeze bypass; a freeze holder merges with
`pr merge --allow-merge-freeze`. A GitHub Enterprise Server whose schema has no
merge queue is treated as queueless, and only its freeze list is read.

PR/MR labels `state::do-not-merge`, `do-not-merge`, and `control::hold` block
both direct merge and queue enqueue with `pr_hold_active`, independently of
repository freezes. Only authorized label removal lifts a hold; new heads and
freeze overrides do not. Labels are read completely from the provider on entry
and immediately before merge/enqueue. Read failures block with
`pr_hold_labels_unavailable`. Add custom names through `[merge].hold_labels`
in repository or user-global config; the compatibility defaults always apply.

When the base branch requires a merge queue, `pr merge` still runs every gate.
It then enqueues the verified head with `enqueuePullRequest` instead of calling
the direct merge API, and waits for the queue to land it, bounded by
`--queue-timeout <seconds>` (default 2700). A PR that is already queued or
already merged is not enqueued again, so a rerun resumes the wait. Without
`--method`, `pr merge` and `pr deliver` use the queue's method. Failures are
typed:

- `merge_queue_enqueue_rejected`: the enqueue mutation was rejected.
- `merge_queue_poll_failed`: a queue poll returned GraphQL errors.
- `merge_queue_dequeued`: the PR was closed, or stayed open with no queue
  entry through a 60-second grace. The provider can drop the entry of a PR it
  merged before the PR reads as merged, so that reading is re-read first. When
  `--queue-timeout` ends the wait inside the grace, the result is
  `merge_queue_timeout` instead.
- `merge_queue_checks_failed`: the queue reports the PR as unmergeable.
- `merge_queue_timeout`: the wait exceeded `--queue-timeout`. The PR is still
  queued and will merge unless it is dequeued; rerun `pr merge` to resume.
- `merge_queue_method_mismatch`: an explicit `--method` differs from the
  queue's configured method.

`pr review --submit-review` requires `--expected-head <sha>` and compares the
provider head before any native review mutation. Summary-only reviews keep the
pending-only ownership guard. Threaded reviews first append an immutable
`forge-cli.review-loop.v1` receipt, then create or resume the exact marked draft,
add only the missing ordered inline-comment suffix, and submit after one final
complete snapshot. Any interruption preserves the pending review; rerunning the
same command resumes it, and an already-submitted run returns idempotent success.

## Inbox discovery

`forge-cli inbox` is a read-only personal work inbox for agents, scheduled jobs,
and Alfred-style consumers:

```sh
forge-cli inbox list --format json
forge-cli inbox status --provider gitlab --gitlab-host gitlab.example.com --format json
forge-cli inbox next --limit 5 --format json
```

With no `--provider`, inbox queries GitHub and GitLab and keeps successful
provider results when another provider fails. GitLab inbox calls always pass
`--hostname <host>` to `glab api`; set `FORGE_CLI_INBOX_GITLAB_HOST` for a
default self-managed host, or use `--gitlab-host` for a per-command override.
`status` reports bounded counts, and `next` returns a ranked bounded subset
without mutating PRs, issues, merge requests, or todos.

For VPN-dependent GitLab hosts, keep daily mixed-provider usage responsive by
requiring a readiness check and bounding GitLab backend calls:

```sh
forge-cli inbox list --format json \
  --gitlab-host gitlab.example.com \
  --gitlab-vpn required \
  --gitlab-vpn-check tcp:gitlab.example.com:443 \
  --provider-timeout 20s
```

When the VPN check fails, mixed-provider mode still returns GitHub results with
a GitLab `vpn_unavailable` provider row and warning. `--provider github`
intentionally skips GitLab. `--provider gitlab` fails when GitLab is selected
but VPN-unavailable or timed out. Add `--strict-providers` for automation that
must fail any partial provider failure.

`--gitlab-vpn-check cmd:<program>` delegates readiness to a local script, and
`--gitlab-vpn-check openvpn` verifies local OpenVPN CLI/profile prerequisites
without starting or stopping VPN. OpenVPN profile paths are local-only
configuration and are redacted from JSON, warnings, issue records, docs, and
cache files. Install optional OpenVPN CLI support with `brew install openvpn`.

Successful provider reads write local cache snapshots. Stale fallback is
opt-in:

```sh
forge-cli inbox list --format json --cache-fallback --cache-max-age 30m
```

Cached fallback items are marked with `stale` metadata and the provider row
remains `ok=false`, so consumers can distinguish stale context from live data.

## Activity discovery

Personal activity commands (`activity commits`, `activity events`, and
`activity summary`) report GitHub user activity. `activity feed` is
repository/project-scoped and supports GitHub plus GitLab:

```sh
forge-cli --provider github --repo owner/name activity feed --since 2026-06-01 --format json
forge-cli --provider gitlab --repo group/project activity feed --since 2026-06-01 --format json
```

Feed rows expose common `kind` / `action` fields for scanning and keep
provider-native semantics in `provider_event_type` plus `details`, so GitHub
and GitLab event differences are preserved instead of flattened.

### Reason filter (`--kind`) vs item-type filter (`--item-type`)

`--kind` selects inbox *reasons* — why an item should appear (`review`,
`assigned`, `todo`, `authored`, `involved`). `--item-type` selects *result
classes* — pull/merge requests, issues, or all items. They are independent:

```sh
# default: all reasons, all item types
forge-cli inbox list --format json

# pull/merge requests only (skips GitHub issue searches and GitLab issue API calls)
forge-cli inbox list --item-type pr --format json

# issues only (skips PR searches; GitHub review-requested is dropped)
forge-cli inbox list --item-type issue --format json

# review-requested PRs only
forge-cli inbox list --kind review --item-type pr --format json
```

`--item-type` defaults to `all`. Dry-run output reflects the pruned query plan:

```sh
forge-cli --dry-run --format json inbox list --item-type pr
```

GitLab `todos` are classified by `target_type` (or the target URL); todos whose
target cannot be classified appear only in `--item-type all` mode.

## Label catalog operations

`forge-cli label` keeps provider labels aligned with a caller-owned YAML/JSON
catalog. The catalog remains outside `nils-cli`; `forge-cli` only validates,
audits, and applies the provider operations.

```sh
forge-cli label list --format json
forge-cli label audit --catalog manifests/forge-labels.yaml --format json
forge-cli --dry-run label ensure --catalog manifests/forge-labels.yaml --update-existing --format json
```

`label audit` reports missing catalog labels, color / description drift, and
unknown shared labels. `label ensure` creates missing labels and updates
existing color / description drift only with `--update-existing`; it never
deletes labels or renames labels by default.

`pr create` and `pr deliver` accept repeated `--label <name>` flags. Add
`--label-catalog <path> --strict-labels` when the caller wants `forge-cli` to
reject unknown, not-applicable, or mutually exclusive labels before a PR/MR is
opened.

### Latency notes

Provider adapters and independent query families run concurrently, so
default-mode latency is bounded by the slowest single backend call rather than
their sum. Identity lookup is only issued when a remaining GitLab query needs
it. Manual smoke timings (provider/network dependent, not a CI assertion):

```sh
time forge-cli --provider github --format json inbox list --limit 30
time forge-cli --provider github --format json inbox list --limit 30 --item-type pr
time forge-cli --provider gitlab --gitlab-host gitlab.example.com --format json inbox list --limit 30
time forge-cli --format json inbox list --gitlab-host gitlab.example.com --limit 30
```

Wall-clock latency depends on `gh`/`glab` and remote API responsiveness; treat
these timings as delivery evidence, not deterministic budgets.

## Search

`forge-cli search` runs free-text and reverse-reference queries the structured
`issue list` / `pr list` filters cannot express. It delegates to the provider
search primitives and builds no index. GitHub-only in v1; GitLab and Local
return a structured `provider_unsupported` error, never a silent empty result.

```sh
# Full-text issues / PRs (default --match title,body,comments), single-repo scoped
forge-cli search issues "ratelimit retry" --format json
forge-cli search prs "cache" --match title --limit 10 --format json

# Reverse reference: which issues/PRs reference this ref?
forge-cli search refs-to 123 --format json
forge-cli search refs-to owner/name#123 --format json
forge-cli search refs-to https://github.com/owner/name/pull/123 --format json

# Preview the exact backend argv without calling the provider
forge-cli --dry-run --format json search issues "term"
```

Role split: `issue list` / `pr list` filter by structured fields within one
repo, `inbox` is the personal cross-repo work queue, and `search` is full-text
(`issues` / `prs`) and reverse-reference (`refs-to`) query. The repo slug comes
from `--repo owner/name` or the detected remote. `search issues` / `search prs`
emit `cli.forge-cli.search.issues.v1` / `...search.prs.v1`; `search refs-to`
emits `cli.forge-cli.search.refs-to.v1`. Every hit is the shared `SearchItem`
(`kind`, `number`, `url`, `title`, `state`, `repo`, `matched_field`).

## Program tracker maintenance

`forge-cli issue tracker` keeps a program tracker issue consistent without
rewriting its whole body. A tracker body holds a `## Phase table` of checkbox
rows and a `## Dependency graph` section whose `mermaid` block is derived from
those rows. The row grammar is owned by the `agent-runtime-kit` repository;
`forge-cli` implements it and replays its conformance corpus.

```sh
# Report findings; any finding exits 65 and is listed under data.findings[]
forge-cli issue tracker lint 123 --format json
forge-cli issue tracker lint 123 --check-state --format json

# Print the generated Mermaid block, or write it into the issue body
forge-cli issue tracker graph 123
forge-cli issue tracker graph 123 --write --format json

# Serialize the rows (lanes, references, done flags, dependencies) as JSON
forge-cli issue tracker show owner/repo#123 --format json

# Tick one row, record the delivering PR, and post one comment
forge-cli issue tracker tick 123 --item S2 --pr owner/repo#45 \
  --comment-file note.md --format json

# Work on a local draft: no provider call
forge-cli issue tracker lint --body-file tracker.md --format json
forge-cli issue tracker graph --body-file tracker.md --write
```

- `show` reads the tracker once and lists its valid rows plus the tracker's
  `title`, `state`, and `url`; row findings are reported without failing, so a
  board still shows the lanes of an imperfect tracker.
- `lint` reports the grammar findings (`malformed-row`, `duplicate-id`,
  `unknown-dependency`, `self-dependency`, `cycle`, `stale-graph`) and
  `missing-tracking-label` when the issue lacks `workflow::tracking`.
  `--check-state` reads each referenced issue and adds `state-mismatch` for a
  row whose checkbox disagrees with its issue, and `unreadable-ref` for a
  target that does not exist or is not accessible. A throttled or
  unauthenticated provider is the command's error (exit 69), not a finding.
- The work a tracker body can ask for is bounded. A phase table with more than
  500 rows is not analysed: `lint` reports only `too-many-rows`, `graph`
  refuses with it, and `tick` refuses with `tracker_too_many_rows`.
  `--check-state` reads at most 200 distinct issues; above that it reads none
  and reports `too-many-refs`.
- `graph` refuses when the phase table has row findings, because such a table
  has no generated graph. `--write` changes only the block in the
  `## Dependency graph` section, inserts the block or the section when it is
  missing, and writes nothing when the block is already current. A body without
  a `## Phase table` section is not a tracker, so `--write` refuses it with
  `tracker_no_phase_table` instead of adding a graph to an unrelated issue.
- `tick` changes only the one row line. It refuses an unknown, duplicated, or
  malformed item, and a row that is already ticked with nothing new to record
  is a no-op that posts no comment. When the comment call fails after the row
  was written, the error is `tracker_comment_not_posted`; post the comment with
  `issue comment`.
- Every write reads the issue immediately before writing and transforms that
  fresh body, so two sessions updating different rows of one tracker do not
  overwrite each other the way a whole-body `issue edit --body-file` can. The
  providers offer no compare-and-swap, so a write that lands between the read
  and the write can still be lost.
- `--dry-run` reports the planned change without writing.

See the contract for the payload fields, error codes, and the exact write
rules.

## GitHub checks compatibility

Starting with `nils-cli` `0.17.0`, GitHub `pr checks` calls request only the
`gh 2.92.0` supported JSON fields. Required-check gates use an explicit
`gh pr checks --required` snapshot instead of the removed `isRequired` JSON
field, so `pr checks`, `pr wait-checks`, `pr merge`, and `pr deliver` share the
same compatibility path. If `gh pr checks` fails on a GitHub
`statusCheckRollup` permission traversal, `forge-cli` falls back to
`gh pr view --json headRefOid,statusCheckRollup` and returns the readable
head-SHA rollup rows instead of surfacing a backend error. If that fallback
cannot recover required-check classification, required-only snapshots fail
closed by gating every readable row, synthesize a pending required row when the
readable rollup is empty, and include
`github_status_rollup_requiredness_unknown_all_rows_gated` in `data.warnings[]`.

## GitLab MR delivery compatibility

GitLab MR delivery uses structured API data where `glab` subcommands expose
stable API access but not stable text output:

- `pr checks <iid>` and `pr wait-checks <iid>` read `glab mr view -F json`,
  then `glab api --hostname <host> projects/<project>/pipelines/<id>/jobs` for
  job rows. This path is not blocked by `glab --version` parser ranges.
- Branch-only check snapshots without a repo/project path still use the
  `glab ci status -b <branch>` text-parser fallback and keep the `glab`
  version guard.
- `pr merge <iid>` keeps the existing clean-worktree, draft, default-branch,
  merge-method, branch-cleanup, and required-check gates, then performs the
  GitLab mutation through `glab api --method PUT .../merge` with the MR head
  `sha` when available.
- `pr deliver` inherits the same GitLab checks/wait/merge atoms; there is no
  separate GitLab macro.

## Deterministic linked-issue closeout

After a successful merge, `pr deliver` runs one more step — `issue_closeout` —
that closes every issue the PR references through a `Closes/Fixes #N` closing
keyword. GitHub records those references on the PR as `closingIssuesReferences`
(also surfaced on `pr view` as `closing_issue_refs`), and normally auto-closes
them on merge. That auto-close is **asynchronous**: it can lag the merge by more
than the few seconds a delivery flow takes to check the issue, so a post-merge
"is it closed yet?" probe frequently sees the issue still `OPEN` even though the
link is correct (sympoies/nils-cli#1052). The closeout step removes that
ambiguity by, for each still-open referenced issue, issuing one explicit,
idempotent `issue close --reason completed`. It is a determinism layer over
GitHub's eventually-consistent auto-close, not a workaround for a broken link.

- The step reports one outcome per issue: `closed` (it was open and we closed
  it), `already_closed` (GitHub's auto-close — or a manual close — already
  landed; left untouched), or `error` (the state check or close failed).
- It is **best-effort**: the merge has already landed, so a fetch/close failure
  records an `ok:false` step but never fails the delivery.
- It only ever acts on genuine closing keywords. Issue and dispatch flows
  that link issues with a non-closing `Refs #N` produce an empty
  `closingIssuesReferences`; close those issues explicitly after delivery.
- GitLab is a no-op today: `glab mr view` does not expose the closes-issues
  connection, so `closing_issue_refs` is always empty there.
- Pass `--no-issue-closeout` to skip the step entirely.

Designated-review delivery uses `pr review-handoff assign|inspect|check|surrender|recover`.
It requires a current-head published review and a reviewer-owned closed ledger;
mailbox verdicts never replace provider evidence. See the
[handoff contract](docs/specs/forge-cli-spec-v1.md#pr-review-handoff-assign--inspect--check--surrender--recover)
for assignment, writer fencing, generation-bound surrender/recovery, and privacy boundaries.
A successor can recover a retired coordinator with `--reason coordinator-retired`,
the recorded `--coordinator-session`, and exact `--base-sha`, `--expected-head`,
and `--expected-state`. Fresh terminal session-board evidence is required;
the transfer preserves findings and keeps review verdicts reviewer-owned.
