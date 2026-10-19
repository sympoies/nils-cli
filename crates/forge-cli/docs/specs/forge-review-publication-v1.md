# forge-cli: Review publication and thread writes

Part of the [forge-cli v1 contract](forge-cli-spec-v1.md). Shared provider
parity, naming, [lock-down rules](forge-cli-spec-v1.md#lock-down-policy),
[exit codes](forge-cli-spec-v1.md#exit-code-map), and
[configuration](forge-cli-spec-v1.md#configuration) remain in the main spec.
The [operation catalog](forge-cli-ops-v1.yaml) owns backend mappings,
validation rules, and output schema versions.

## Operations

### `pr review`

- The `pr review <id>` posting surface emits
  `cli.forge-cli.pr.review.v1`. It accepts an already-rendered review outcome
  body via `--comment <text>` or `--comment-file <path>`, plus a
  `--decision comments-only|approve|request-changes` metadata value and
  repeatable `--lens <name>` entries.
- By default `--decision` is recorded in the envelope and generated issue mirror
  body only (the outcome-comment form); it does not call provider-native
  approve/request-changes APIs.
- `--specialist-report` opts the supplied body into the canonical
  `agent-kit:specialist-review-report:v1` contract. Before any provider call,
  forge-cli requires exactly one marker and `## Review Report` heading,
  non-empty Reviewable/Lens/Lens verdict/Scope/Evidence reviewed fields, the
  closed `pass|findings|blocked|follow-up-pass` verdict vocabulary, and the
  canonical findings table with at least one exactly-five-cell row. Generic
  review comments omit this flag and remain valid.
  On GitHub, the report must contain exactly one Reviewable value identifying
  the selected PR: its URL, `PR #N`, `#N`, or `owner/repo#N`, optionally
  followed by a space and `at <full commit SHA>` (40 hexadecimal digits).
  References
  are compared exactly. Offline validation checks the qualifier syntax;
  publication with `--expected-head` also requires it to match that head.
  A target, syntax, or head mismatch returns `reviewable_mismatch`
  (`DATA 65`) before provider access. Non-GitHub publication keeps the
  canonical format checks without this GitHub target binding.
  Non-empty is not sufficient: a field whose whole value is one of the
  renderer's placeholders (`not provided`, `unspecified`, `no input files`) is
  rejected with `invalid_specialist_review_report`. `review-specialists` no
  longer emits any of them for a publication-bound profile; this guard is the
  second line for a body produced by an older binary or built by hand. A real
  value that merely contains the word — a scope of `behavior left unspecified
  by the spec` — still passes, because the match is on the whole trimmed field.

  One renderer fallback is deliberately not covered. When `--evidence-reviewed`
  was absent, the old renderer also substituted the joined list of input file
  paths, which has no fixed spelling and is indistinguishable from a legitimate
  citation of those same files. Rejecting it here would need a heuristic; the
  renderer refusing to emit it is the durable fix.
- `--metadata-only --expected-head <sha> --native-review-url <url>
  --native-review-author <login>`
  (GitHub-only) records concise
  personal-identity metadata after an owner App has already published the
  authoritative native review. The URL must identify a
  `#pullrequestreview-<id>` object on the selected repository and PR. Before
  mutation, forge-cli verifies the PR still has `--expected-head`, then reads
  that exact review back and verifies its URL, decision state, App author
  login, and `commit_id`. The PR
  comment and optional issue mirror contain only PR, decision, lenses, and the
  native review link; they never include the native Review Report body.
  Metadata-only mode rejects caller bodies, `--specialist-report`, threads, and
  `--submit-review` before mutation. Omitting the expected head returns
  `expected_review_head_required`; provider-head drift returns
  `github_review_head_changed`; a review bound to another commit returns
  `native_review_verification_failed`. Output adds `data.metadata_only=true`,
  `data.head_sha`, `data.native_review_url`, and `data.native_review_author`.
- With `--submit-review --expected-head <sha>` (GitHub-only in v1) the command
  instead submits a native
  pull request review event: it POSTs `gh api repos/{repo}/pulls/{id}/reviews`
  with `--decision` mapped to the review `event` and the reviewed head bound as
  `commit_id`
  (`comments-only→COMMENT`, `approve→APPROVE`, `request-changes→REQUEST_CHANGES`),
  creating the `#pullrequestreview-` object reported as `data.pr_comment_url`
  with `data.submitted_review = true`. The review is authored by whatever
  identity the inherited `gh` token carries, so a reviewer-bot token (for example
  via `FORGE_BOT_PROFILE`) yields a bot-authored review. A body is required for
  `COMMENT` and `REQUEST_CHANGES` and optional for `APPROVE` (a body-less approve
  omits the `body` field). The same PR-existence guard runs first. For a
  summary-only native review it is followed by a complete pending-only review
  snapshot; threaded reviews use the receipt transaction below. The provider
  head must still equal
  `--expected-head`; drift returns `github_review_head_changed` (`DATA 65`)
  before any mutation. The successful payload exposes the bound head as
  `data.head_sha`. If the authenticated viewer already
  owns a pending review during a summary-only submission, the command returns
  `github_pending_review_exists` (`RUNTIME 1`) before any review mutation. Its
  detail includes the provider head, viewer-owned pending count, and deletable
  count so callers can inspect the exact node. Pending reviews owned by other
  viewers do not block submission.
- `--recover-pending` turns that conflict into a recovery instead of a stop: the
  abandoned viewer-owned pending review this submission would have replaced is
  deleted, then the submission proceeds. Because the conflict is raised by a
  *preflight*, before any mutation, there is no half-applied review to reconcile
  and no retry to sequence — clearing the node is the whole recovery. What
  remains is the guard around an unundoable delete, so it is strict: recovery is
  attempted only when the PR owns exactly one viewer-authored pending review the
  viewer may delete, and the delete itself then re-proves, under the
  cross-process lease, that the node is still bound to `--expected-head`, carries
  no inline draft comments, and has a body byte-identical to the body being
  submitted. That is the same guard, lease, and post-delete read-back as
  `pr pending-review delete --confirm-abandoned`, because it is the same code
  path. A candidate that fails one of those guards surfaces that guard's own
  `pending_review_*` error rather than the conflict, so the refusal says which
  assumption did not hold; only the absence of a single nameable candidate hands
  back `github_pending_review_exists` unchanged. A successful recovery reports
  the deleted node as `data.recovered_pending_review`, and the flag is refused
  before any provider call with `recover_pending_requires_submit_review`
  (`DATA 65`) when there is no native submission for it to guard.
  The pending guard and reviews POST are
  rendered in `--dry-run` as
  `data.pending_review_guard_plan` and `data.plan`. Omitting the expected head
  returns `expected_review_head_required` (`DATA 65`); supplying it without
  `--submit-review` or `--metadata-only` returns
  `expected_review_head_requires_submit_review` (`DATA 65`). `--submit-review` on
  GitLab / Local returns `provider_unsupported` (`USAGE 64`).
  If GitHub rejects the native review submission with HTTP 422, the command
  returns `github_native_review_rejected` (`RUNTIME 1`) and preserves the raw
  backend detail plus retry guidance; this covers identities that can comment
  but are not eligible to submit an approval review, such as some GitHub App bot
  identities.
- With `--thread-file <path>` (GitHub-only in v1), the command creates
  resolvable review threads for actionable findings. The file must be a JSON
  array of entries shaped like
  `{ "path": "src/lib.rs", "line": 42, "side": "RIGHT", "body": "..." }`.
  `side` defaults to `RIGHT`; `startLine` / `startSide` are accepted for ranged
  comments; omitting `line` creates a file-level thread (`subjectType=FILE`),
  while line comments use `subjectType=LINE`. The thread file is capped at
  256 KiB, 50 specs, 1024-byte paths, and 16 KiB bodies; put lower-priority
  findings in the summary body or split them into a later review. `--thread-file`
  requires `--submit-review`; omit it for a summary-only review. A live GitHub
  run first reads a lightweight, fully paginated root-comment fingerprint of
  review threads, skips findings
  whose semantic `(path, body)` already has a live non-resolved/non-outdated
  thread, computes a deterministic `review_run_id`, and appends an immutable
  `forge-cli.review-loop.v1` receipt before native-review mutation. It then
  takes the same private provider/host/repository/PR/viewer cross-process lease
  used by pending-review recovery and selects only an exact viewer-owned
  receipt-bound pending review, or creates a
  new review bound to `--expected-head` through `commitOID`. The review body and
  every finding carry owned run/digest markers. An exact pending manifest must
  be an ordered prefix of the receipt manifest; the command adds only the
  missing suffix and performs a final complete snapshot before
  `submitPullRequestReview`. During snapshot recovery, GitHub's equivalent
  single-line encodings (`startLine` omitted or equal to `line` without a
  distinct start side, and likewise for the original line) are canonicalized
  before comparing the comment with its review thread; genuine ranges,
  including equal-number cross-side ranges, must still match exactly. Success
  requires an immediate exact-node read-back. Unknown sides in the pending
  comment never prove a single-line anchor; the exact thread read owns that
  decision.
  whose submitted state and complete receipt-bound manifest reconcile.

  Every interrupted stage is resumable: a lost create response, any lost inline
  comment response, a pre-submit failure, and a lost submit response preserve
  completed content. Re-running the same immutable inputs resumes or returns
  the authenticated viewer's already-submitted review by `review_run_id`;
  submitted-review history is scanned only when the exact immutable receipt
  already exists, and no automatic path deletes a
  draft. A unmarked, ambiguous, different-head/commit/identity, or manifest-mismatched
  pending review fails closed before mutation. An outdated semantic thread match
  is posted fresh.
  `data.threads_skipped_idempotent` reports the number skipped, and when every
  finding is already threaded the review event itself is skipped
  (`submitted_review` is `false`). An exact completed rerun also reports
  `submitted_review: false` while retaining the original submitted review URL;
  it does not emit another issue mirror activity entry. JSON output includes
  `data.review_threads[] = { id, url, path, line, subject_type }`, where `id` is
  the `PRRT_...` handle consumed by `pr review-threads resolve`. Dry-run output
  includes `data.target_plan`, `data.thread_plan[]`, `data.submit_plan`,
  `data.review_state_plan`, `data.review_receipt_plan`,
  `data.review_state_verify_plan`, and `data.planned_review_threads`. If GitHub
  rejects an individual thread mutation
  with HTTP 422 because the path/line is not commentable on the diff, the command
  returns `github_review_thread_rejected` (`RUNTIME 1`) with the raw backend
  detail and the failed spec index/path/line while preserving the pending
  review. Other interruption points return
  `pending_review_transaction_incomplete` (`DATA 65`) with the pending review
  intact and an identical rerun as the recovery action. A lost creation response
  triggers a read-only lookup for the exact own receipt-bound transaction;
  unprovable identity stays unknown and never authorizes deletion. Once the review ID is
  known, error details include exact inspect commands plus fresh-snapshot shell
  commands (POSIX shell and `jq`) for guarded `resume-submit` (only for a complete manifest) and
  explicit `discard --confirm-discard --confirm-inline-content-loss`. These
  commands preserve the original provider host and repository. Snapshot read
  or validation failures after thread creation retain the transaction ID and
  this same guidance, including on an identical rerun. Existing recovery guard
  failures keep their specific validation codes while gaining this context.
  These failures never delete the partial review. Malformed or oversized
  specs return `invalid_review_thread_spec` (`DATA 65`); `--thread-file` without
  `--submit-review` returns `thread_file_requires_submit_review` (`DATA 65`);
  GitLab / Local return `provider_unsupported` (`USAGE 64`) before any backend
  call. Findings that cannot be mapped to a changed file/line should stay in
  the summary review body instead of being forced into a thread spec.
- On GitHub, PR comments use the issue-comments API endpoint so JSON output can
  report the created comment URL directly: `data.pr_comment_url`. Because that
  endpoint accepts both issues and pull requests, the command first verifies
  `<id>` is a pull request (`gh api repos/{repo}/pulls/{id}`) before posting — so
  a typo'd or non-PR number can never silently post a review outcome onto an
  unrelated issue. Only a `404 Not Found` from that guard yields
  `id_not_pull_request` (`DATA 65`); any other non-zero result (rate limit, 5xx,
  forbidden/SSO) surfaces as a retryable `backend_error` (`RUNTIME 1`) since it
  may have hit a valid PR. The guard read is also rendered in `--dry-run` output
  as `data.guard_plan`. On GitLab, a single `glab mr note create --help` probe
  selects the review-note form across every `glab` version class: when
  `--resolvable` is advertised it posts a non-resolvable status note
  (`glab mr note create … --resolvable=false`) so it does not register as an
  unresolved MR discussion that blocks the next `forge-cli pr merge`; when the
  `create` subcommand exists but lacks `--resolvable` it drops only that flag
  (`glab mr note create … --message`, which stays resolvable); and when the build
  has no `mr note create` subcommand at all it uses the bare
  `glab mr note <id> --message` form. If the backend prints a URL, it is
  surfaced as `data.pr_comment_url`.
- With `--mirror-issue --issue <number>`, the command posts a compact issue
  activity comment linking to the PR review comment and reports
  `data.issue_comment_url` when the backend returns one. `--mirror-issue`
  without `--issue` returns the `issue_required` (`DATA 65`) envelope at runtime
  (the CLI does not impose a clap parse-time requirement, so JSON consumers can
  branch on the error kind). The generated mirror body references the PR/MR with
  the provider-correct sigil (`#<n>` on GitHub, `!<n>` on GitLab). Its
  user-controlled `--lens` content is run through the same `no_local_path` and
  `no_escaped_control_markdown` guards as the review body, and that validation —
  plus the `--issue` requirement — is enforced before any backend post, so a
  rejected mirror can never leave a posted review outcome with no mirror.
- Output schema:
  `data = { provider, number, decision, submitted_review, head_sha?,
  pr_comment_url, issue_number, issue_comment_url, mirrored, lenses,
  metadata_only?, native_review_url?, native_review_author?,
  native_review_verification_plan?,
  review_threads?, threads_skipped_idempotent? }`. `head_sha` is present for
  `--submit-review` and `--metadata-only`; `threads_skipped_idempotent` is present (non-zero) when
  cross-run duplicates were skipped.

### `pr review-dismiss`

- `pr review-dismiss <id> --review-id <id> --expected-head <sha>
  --reason-file <path>` dismisses one native GitHub review and emits
  `cli.forge-cli.pr.review-dismiss.v1` with `review_id`, `review_url`,
  `author`, `head_sha`, `state`, `reason`, `actor`, and `dismissed_at` (when
  forge-cli observed the provider confirm the dismissal).
- It refuses with exit 65 unless every guard holds: a non-empty reason
  (`review_dismiss_reason_missing`, checked before any provider call); the PR
  head and the review commit both equal `--expected-head`
  (`review_dismiss_head_mismatch`); the review is `CHANGES_REQUESTED`
  (`review_dismiss_state_invalid`); and the author is a provider App account
  whose login is listed in the user-global `[[review_convergence.bots]]`
  (`review_dismiss_author_not_automated`). Repository `.forge-cli.toml` never
  grants dismissal authority, because a PR branch can edit it. A login spelling
  never makes an author automated, and a configured login on a user account is
  refused.
- Dismissal never replaces designated review. The PR must hold a designated
  PASS admitted at `--expected-head` by the same gate as `review-handoff check`
  (`awaiting_designated_review`, also returned when no handoff exists or a
  blocking ledger finding is open). The review must be submitted before that
  PASS, so the designated reviewer could judge it
  (`review_dismiss_after_designated_pass`). The designated reviewer's own review
  is refused (`review_dismiss_designated_reviewer`). A valid `AGENT_SESSION_ID`
  is required, and neither the designated reviewer session nor the designated
  review account may dismiss (`review_dismiss_self_review`).
- The reason, head, and authenticated actor are posted as the provider
  dismissal message, which GitHub retains with the review. No review-loop
  ledger record is appended, so existing ledger readers stay compatible.
  `--dry-run` runs every guard and prints the dismissal plan. An unreadable
  actor (`review_dismiss_actor_unknown`) or an unconfirmed provider response
  (`review_dismiss_unconfirmed`) fails with exit 69. GitLab and local providers
  return `provider_unsupported` (exit 64).

### `pr review validate`

- The `pr review validate [id]` preflight surface emits
  `cli.forge-cli.pr.review.validate.v1` and never posts provider-visible review
  activity. It accepts `--comment <text>` / `--comment-file <path>` and
  `--thread-file <path>` using the same body limits, JSON shape, local-path
  guard, escaped-control guard, and size limits as `pr review`.
- With `--specialist-report`, the preflight also applies the canonical marker,
  field, verdict, and findings-table validation described above. A mismatch
  returns `invalid_specialist_review_report` (`DATA 65`) before diff or provider
  access. On GitHub when an `id` is supplied, the same target binding as
  publication applies and a target mismatch returns `reviewable_mismatch`
  (`DATA 65`). Without a repository slug, the local preflight accepts only
  `PR #N` and `#N`, with the same optional full-SHA syntax check; it does
  not require a repository or read a provider to
  build the URL and repository-qualified forms. Without an `id`, or for a
  non-GitHub provider, only canonical format/content validation applies.
- Without `--check-diff`, validation is local-only and works for GitHub, GitLab,
  and Local provider contexts. This is the format/content dry-run path for
  agents that want to validate `review-report.md` and `review-threads.json`
  before a native review submission.
- With `--check-diff`, an `id` is required and the command is GitHub-only in v1.
  It fetches `gh api repos/{repo}/pulls/{id}/files --paginate`, parses each
  patch hunk, and verifies every line thread maps to a changed line on the
  requested `side` (`RIGHT` default, `LEFT` supported). Ranged threads must keep
  their start and end coordinates in one patch hunk with start before end.
  File-level threads only require the file to be part of the PR file list. A
  missing file returns `review_thread_file_not_changed` (`DATA 65`); a
  non-commentable line returns `review_thread_line_not_in_diff` (`DATA 65`); a
  reversed or cross-hunk range returns `review_thread_range_not_in_diff`
  (`DATA 65`). With global `--dry-run`, `--check-diff` does not invoke GitHub:
  JSON output includes `data.diff_plan` and
  `data.review_threads.diff_checked=false`.
- JSON output includes
  `data = { provider, number?, check_diff, comment, review_threads }`, where
  `comment = { present, bytes, lines, specialist_report }` and
  `review_threads = { count, diff_checked, specs[] }`. Each normalized
  `specs[]` entry includes `{ index, path, line?, side, start_line?, start_side?,
  subject_type, body_bytes }`.

### `pr review-threads resolve` / `pr review-threads reply`

- GitHub-first write surfaces over a single review thread, keyed by the
  thread `id` from the read surface. GitLab and Local have no
  GitHub-shaped thread-mutation surface, so both return
  `provider_unsupported` (`USAGE 64`) before any backend call.
- Live GitHub writes first verify that `--thread <thread_id>` belongs to the
  positional PR id. A mismatch fails with
  `review_thread_pr_mismatch` (`DATA 65`) before any reply or resolve mutation.
  `--dry-run` remains offline and does not perform this validation lookup.
- `pr review-threads resolve <id> --thread <thread_id>
  [--note <text> | --note-file <path>]`:
  - Reads the root comment database id and resolution permission. With
    `--note` / `--note-file`, posts a REST reply to
    `repos/{repo}/pulls/{id}/comments/{root_comment_id}/replies`, then resolves
    via GraphQL `resolveReviewThread`. Without a note, only resolves.
    Missing resolution permission fails with `review_thread_resolve_forbidden`
    before posting a note; an invalid reply target fails with
    `review_snapshot_incomplete`.
  - Idempotent: `resolveReviewThread` succeeds on an already-resolved
    thread, so resolving twice is success rather than an error.
  - Output schema: `cli.forge-cli.pr.review-threads.resolve.v1`,
    `data = { provider, thread_id, resolved, replied }`.
- `pr review-threads reply <id> --thread <thread_id>
  --body <text> | --body-file <path>`:
  - Reads the root comment database id and posts a REST review-comment reply;
    never resolves the thread. An invalid root target fails with
    `review_snapshot_incomplete`.
  - Output schema: `cli.forge-cli.pr.review-threads.reply.v1`,
    `data = { provider, thread_id, comment_url }`.
- The `no_local_path` privacy guard runs over the reply note / body
  before the backend call, matching `pr comment`.
- GitHub may wrap a REST reply in an empty native `COMMENTED` review. The
  designated admission check preserves an earlier canonical approval only
  after verifying this empty review contains no root inline comments.
- Offline dry-run output includes `target_plan`, `root_comment_id_source`, and
  the mutation plan with a `${root_comment_id}` placeholder; it does not claim that a
  live reply target or resolution permission has been validated.
