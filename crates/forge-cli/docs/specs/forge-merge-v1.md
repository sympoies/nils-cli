# forge-cli: Pull request merge

Part of the [forge-cli v1 contract](forge-cli-spec-v1.md). Shared provider
parity, naming, [lock-down rules](forge-cli-spec-v1.md#lock-down-policy),
[exit codes](forge-cli-spec-v1.md#exit-code-map), and
[configuration](forge-cli-spec-v1.md#configuration) remain in the main spec.
The [operation catalog](forge-cli-ops-v1.yaml) owns backend mappings,
validation rules, and output schema versions.

## Operations

### `pr merge`

- Preconditions enforced before invoking the backend:
  - PR exists and is not draft (call `pr ready` first if needed);
  - working tree clean;
  - required checks all green **and at least one check reported**
    (re-checked even if `pr wait-checks` succeeded earlier — TTL-zero
    gate). A head with no required checks and no visible rows is refused
    with `checks_not_registered`; `--allow-no-checks` with a non-empty
    `--allow-no-checks-reason` bypasses it and records the reason in
    `data.no_checks_override_reason`. `pr deliver` accepts the same pair
    and forwards it to its wait-checks and merge steps. Without the flags,
    a `[checks] none = true` declaration with a non-empty `none_reason` has
    the same effect and records that reason in the same field — read from
    the `.forge-cli.toml` committed on the base branch
    (`refs/remotes/<remote>/<base>`), never the working tree, so a PR head
    cannot waive its own gate;
  - target branch is the repo default branch OR explicitly approved
    via `--allow-non-default-base`;
  - when resolved review convergence is enabled, no current-head native
    `CHANGES_REQUESTED`, and any already-observed configured bot activity has
    stayed quiet for the resolved quiet period; the initial provider view must
    also expose a non-empty head OID or the merge fails closed with
    `review_convergence_head_missing`;
  - existing review-loop ledgers are gated independently of the quiet-
    convergence override: the latest typed state is bound to the exact merge
    head, has no active hard stop, and has no open blocking findings. Enabling
    review convergence requires an explicit genesis observation when no ledger
    exists. Explicit `--review-convergence=false` permits an absent observation
    even when a designated-review handoff exists; it does not waive an existing
    observation or its designated-publication checks. The complete trusted
    chain is still validated. The chain tip must remain unchanged through the final pre-merge
    recheck; there is no force/rationale bypass for existing state;
  - no non-outdated unresolved review threads (outdated unresolved threads
    are mechanically dispositioned `stale` and recorded, not blocking) OR
    the remaining live threads explicitly bypassed via
    `--allow-unresolved-threads` with a recorded
    `--allow-unresolved-threads-reason`;
  - no unchecked task-list items in the description OR explicitly
    bypassed via `--allow-unchecked-tasks` with a recorded
    `--allow-unchecked-tasks-reason` (the description is the delivery
    contract: every `- [ ]` is checked off or rewritten as
    dispositioned before merge);
  - on GitHub, no active merge freeze (an open `merge-freeze` issue) OR every
    active freeze named with `--allow-merge-freeze` plus a recorded
    `--allow-merge-freeze-reason`. A base branch that requires a merge queue
    lands through `enqueuePullRequest` instead of the direct merge API (policy
    18);
  - `--method squash|merge|rebase` (default `squash`, configurable
    per repo).
- Direct callers may pass `--expected-head <sha>`. The first provider snapshot
  must still expose that exact head or the command returns
  `test_first_evidence_provider_head_mismatch` before any merge mutation. The
  same reviewed head remains bound through the provider merge compare-and-swap.
- Direct callers may pass `--expected-base <branch>`. The first provider
  snapshot must expose that exact target or the command returns
  `delivery_base_mismatch` before any merge mutation.
- With convergence enabled, complete native-review and review-thread/comment
  snapshots are fetched once more after the thread/task gates and immediately
  before the provider merge. A late request blocks; any semantic digest change,
  including removal or cleanup after the quiet snapshot, returns
  `review_convergence_activity_changed` so the caller can rerun convergence.
- `--dry-run` validates the same enabled-policy provider contract as live
  merge and includes the resolved policy in `data.review_convergence`. GitLab
  therefore returns `provider_unsupported` before either path touches the
  provider. `pr deliver --no-merge` remains exempt because it has no merge
  convergence phase.
- Post-merge: deletes the remote branch (default `true`, disable via
  `--keep-branch`). Before requesting deletion, read all open PRs/MRs targeting
  the source branch in its repository/project. If any dependents remain, merge
  while preserving the branch and report their numbers/URLs in `branch_cleanup`.
  Retarget dependents before subsequent cleanup; the CLI does not retarget them
  automatically.

  `--delete-branch-even-if-stacked` explicitly permits deletion
  and records a warning that dependents may close; it conflicts with
  `--keep-branch`.

  Failed or malformed dependency reads defer the merge with
  `branch_dependencies_unavailable`. GitHub automatic branch deletion with
  dependents defers the merge with `stacked_branch_auto_delete`, since omitting
  the cleanup flag cannot preserve the branch in that configuration, including
  `--keep-branch` and `[merge].delete_branch=false` on GitHub. The same
  dependency guard runs through `pr deliver`; its override defaults to false.
  Dry-run remains a backend plan preview; dependency checks run during live merge.

  GitHub passes `--match-head-commit <head_sha>`;
  GitLab performs the merge mutation through
  `glab api --method PUT projects/<project>/merge_requests/<iid>/merge`
  after all gates pass, including `sha=<head_sha>` when the MR view
  exposes it so the source branch HEAD cannot drift silently between
  checks and merge. If a backend reports failure after another actor completed
  the merge, idempotent recovery succeeds only when the merged head still
  equals that same OID.
- Output schema: `cli.forge-cli.pr.merge.v1`,
  `data = { number, url, merge_sha, method, deleted_branch, branch_cleanup?,
  unchecked_tasks_override_reason?, unresolved_threads_override_reason?,
  no_checks_override_reason?,
  stale_thread_dispositions?, review_convergence?, review_loop? }`.
  `unchecked_tasks_override_reason` / `unresolved_threads_override_reason` /
  `no_checks_override_reason` are
  present only when the matching bypass was used; `stale_thread_dispositions`
  lists the outdated threads dispositioned `stale` at rule 13 (omitted when
  none). The additive convergence snapshot contains
  `required`, `head_sha`, `observed_reviews`, `stale_reviews`,
  `unresolved_threads`, `changes_requested_by`, `missing_reviewers`,
  `latest_activity_at`, `quiet_until`, `quiet_period_ms`, `timeout_ms`,
  `waited_ms`, and the resolved `bots`. It is absent while the feature is off.
