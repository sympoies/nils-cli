# forge-cli: Review transaction and handoff state

Part of the [forge-cli v1 contract](forge-cli-spec-v1.md). Shared provider
parity, naming, [lock-down rules](forge-cli-spec-v1.md#lock-down-policy),
[exit codes](forge-cli-spec-v1.md#exit-code-map), and
[configuration](forge-cli-spec-v1.md#configuration) remain in the main spec.
The [operation catalog](forge-cli-ops-v1.yaml) owns backend mappings,
validation rules, and output schema versions.

## Operations

### Review transaction state and pending-review recovery

- Threaded native reviews use the provider-visible, append-only
  `forge-cli.review-loop.v1` state chain. A record requires `schema`,
  `repository`, `pr`, `expected_head`, zero-based contiguous `generation`,
  nullable `previous_digest`, typed `payload`, and `record_digest`. A
  `review-run-receipt` payload requires `review_run_id`, portable
  `route_lenses`, `decision`, `expected_head`, `round`, `summary_digest`, and an
  ordered `inline_manifest[]` of `index`, `path`, optional line/range anchors,
  side, subject type, and normalized `body_digest`.
- Digests are lowercase SHA-256 values prefixed by `sha256:`. Their preimages
  are compact UTF-8 JSON in the declared field order. `review_run_id` binds
  repository, PR number, expected head, round, semantic route lenses, decision,
  normalized summary digest, and ordered inline manifest. A record digest also
  binds the prior record digest. Byte-identical append retries are deduplicated;
  conflicting duplicates, forks, missing/unreachable generations, target
  mismatches, malformed markers, or digest mismatches fail as
  `review_state_conflict` before native-review mutation. Encoded state markers
  are limited to 64 KiB before provider mutation, and the complete rendered
  comment body — visible text plus marker — is limited to 64 KiB before provider
  mutation. An oversized marker returns `review_state_record_too_large`; an
  oversized complete body returns `review_state_comment_too_large`.
- Provider marker grammar is versioned and deterministic:
  `<!-- forge-cli:review-state:v1 <lowercase-hex-record-json> -->`,
  `<!-- forge-cli:review-run:v1 run=<review_run_id> -->`, and
  `<!-- forge-cli:review-finding:v1 run=<review_run_id> digest=<body_digest> -->`.
  Owned markers remain in raw provider content but are removed before semantic
  body comparison and digesting. State reads retain comments authored by the
  authenticated viewer or a provider-classified `OWNER`, `MEMBER`, or
  `COLLABORATOR`. For the self-author comparison only, a terminal GitHub App
  `[bot]` suffix on the authenticated viewer is canonicalized to the comment
  author form; no other login normalization occurs. Marker-shaped comments
  from unprivileged actors cannot extend or poison the transaction chain. This
  lets a later authorized session resume the same provider-visible chain
  without a machine-local ledger.
- Record encoding and comment presentation are separate. Every state comment the
  CLI posts leads with a record-specific plain-English checkpoint sentence,
  followed by the unchanged marker on its own line.
  GitHub hides HTML comments when it renders Markdown, so a marker-only body
  appears in the timeline as a blank comment under the operator's identity; the
  visible sentence prevents a confusing blank entry without exposing forge-cli's
  generation, payload-kind, or head bookkeeping to ordinary readers. A combined
  delivery outcome is appended *after* both, separated by a
  `---` rule. That ordering is load-bearing, not cosmetic: a Markdown HTML block
  opened in caller text runs until a line containing `-->`, so caller text placed
  above the notice could swallow both the notice and the marker.
- Presentation text never enters a record digest, the chain order, or the parser,
  so historical bare-marker comments and new rendered comments validate as one
  chain with no migration and no edit or deletion of existing comments. The
  chain therefore deduplicates by *record*, not by comment body: two concurrent
  sessions that compute the identical transition converge on one record even if
  their comments differ, so a divergent concurrent outcome is possible and is
  detected only for the writing session (see `review_outcome_not_posted` below).
- The visible checkpoint sentence contains only bounded public author/head,
  review-state, and next-step text. Session identities, digests, and local paths
  stay in the hidden marker and never enter the human-facing sentence.
- Receipt fields intentionally exclude authentication tokens, credentials,
  environment-variable values, local paths, and private identity/profile names.
  The durable identity route contains only portable lens names and the semantic
  decision; environment-owned routing resolves the actor outside the receipt.
- `pr pending-review inspect <id> --review <PRR_...>` emits
  `cli.forge-cli.pr.pending-review.inspect.v1`. Its complete snapshot requires
  PR/head/review identity, author, commit, raw and semantic body,
  `created_at` when supplied by the provider, and an explicit
  `inline_comment_count` in the inspect payload,
  `viewerDidAuthor`, `viewerCanDelete`, provenance, every inline comment with
  normalized body digest plus `diffSide`/`startDiffSide` and line/range anchors,
  and `snapshot_digest`.

  Every page must repeat identical review metadata and a
  stable `totalCount`; partial data, cursor loops, count mismatch, head drift,
  an aggregate above 1,000 comments, or a decoded retained snapshot above 4 MiB
  returns `review_snapshot_incomplete` instead of a partial snapshot. An
  excessive `totalCount` is rejected before another page. Provenance is
  `receipt-bound` or `unmarked`; mixed, missing,
  or digest-mismatched owned markers return `pending_review_manifest_mismatch`.
- `pr pending-review resume-submit <id> --review <PRR_...> --review-run-id
  <digest> --expected-head <sha> --expected-commit <sha> --expected-snapshot
  <digest> --decision <decision>` emits
  `cli.forge-cli.pr.pending-review.resume-submit.v2`. Version 2 makes
  `snapshot_digest` nullable and adds `snapshot_provenance` so idempotent
  recovery never claims a pending snapshot it cannot prove. It submits only an exact
  viewer-owned receipt-bound snapshot after recomputing the run id and matching
  the summary plus every immutable inline-manifest field. If that review was
  already submitted, the command reads submitted reviews and threads, verifies
  the authenticated author, same run id, review id, head, commit, decision
  state, and complete receipt-bound manifest, and returns the same successful
  envelope without another mutation. Because a submitted review cannot prove
  the caller's former pending-snapshot digest, that idempotent envelope reports
  `snapshot_digest: null` with `snapshot_provenance:
  pending-snapshot-unverified`; a live submit reports its guarded digest with
  `snapshot_provenance: pending-cas+submitted-reconciled`. Provider-null
  side/range values on a `FILE` comment are normalized away when matching a
  receipt. For a single-line `LINE` comment, GitHub's equivalent `startLine`
  encodings (omitted or equal to `line` with no distinct start side, and
  likewise for the original line) are canonicalized to omitted before snapshot
  output, receipt matching, and `snapshot_digest` calculation. Non-equal range
  starts, equal-number cross-side ranges, and all other `LINE` anchors remain
  exact. When the pending-comment response cannot establish the line sides,
  canonicalization is deferred until the exact review-thread recovery supplies
  the authoritative sides.
- `pr pending-review submit <id> --review <PRR_...> --expected-head <sha>
  --expected-commit <sha> --expected-snapshot <digest> --decision <decision>
  --confirm-unmarked-submit` emits `cli.forge-cli.pr.pending-review.submit.v1`.
  Its released v1 `snapshot_digest` remains a required string. It accepts only
  `unmarked` provenance and preserves the exact body and all inline comments
  while submitting. It never auto-adopts a unmarked draft.
- `pr pending-review discard <id> --review <PRR_...> --expected-head <sha>
  --expected-commit <sha> --expected-snapshot <digest> --confirm-discard`
  emits `cli.forge-cli.pr.pending-review.discard.v1`. It is destructive and
  requires the additional `--confirm-inline-content-loss` whenever the complete
  snapshot contains an inline comment. No normal `pr review` recovery path
  invokes discard or delete.
- Live inspect data is required for these four commands, so their current v1
  dry-run form fails with `pending_review_snapshot_required`. All mutating forms
  perform the exact head/commit/snapshot and viewer-identity checks under a
  private cross-process forge-cli lease keyed by provider, host, repository,
  PR, and authenticated viewer. Submission and deletion are immediately read
  back and succeed only when the exact submitted manifest or absence is
  reconciled; otherwise they return `pending_review_reconciliation_failed`.
  A busy or unsafe lease returns `pending_review_lease_busy` or
  `pending_review_lease_unsafe`. GitHub exposes no compare-and-swap token on
  review submission, so the lease closes races among cooperating forge-cli
  processes but cannot exclude a same-identity non-cooperating API client in
  the irreducible interval between the final read and mutation. Mismatches return `pending_review_head_changed`,
  `pending_review_commit_mismatch`, `pending_review_manifest_mismatch`,
  `pending_review_identity_mismatch`, `pending_review_pr_mismatch`, or
  `pending_review_not_found` with no mutation.

### `pr review-handoff assign` / `inspect` / `check` / `surrender` / `recover`

The designated-review route is opt-in through `AGENT_REVIEWER_SESSION` or an
explicit `assign` operation. No assignment preserves the existing self-run
results, envelopes, ledger bytes, mutations, and exit codes. Live publication
and delivery read the trusted ledger to detect persisted ownership even after
a restart without the assignment environment. Assigned operations require
GitHub's trusted provider ledger; other providers fail closed when assigned.

- `test-changes <id> --expected-head <sha>` is a read-only pre-review preview
  that needs no assignment or review. It prints test-change findings and the
  declaration-bound confirmation digest; it never admits a review. See
  [test-change admission](test-change-admission-v1.md) for declarations, output
  fields, deterministic limits and recovery.
- `assign <id> --reviewer-session <session-id>[@machine] --review-author <login>
  --base-sha <sha> --expected-head <sha> --expected-state <digest|none>` records
  an explicit handover in the existing `forge-cli.review-state.v1` chain.
  Base and head must match the provider. Coordinator and reviewer must be
  different sessions. Only opaque session digests, the configured public
  native-review author, and commit SHAs reach the provider; selectors and
  machine addresses stay private. Initial assignment generation is one.
  When the provider base moves, the recorded coordinator may repeat `assign`
  for the same reviewer digest and exact public author at the current base,
  head, and retained tip without surrender. This scope refresh advances the
  assignment generation and preserves findings/history. It requires a fresh
  reviewer-owned observation and publication; old-generation writes and old
  reports never satisfy the new interval. A different reviewer or author still
  requires surrender or explicit recovery, as does reassignment at the same base.
  Validation and scope/tip checks run before persistence. A successful live
  assignment returns its verified `handoff_digest` and `state_tip_digest`.
  If the provider acknowledges a handoff write but read-back fails or does not
  expose its record, DATA 65 `review_handoff_persisted_verification_failed`
  explicitly reports `persisted: true`, the written tip in both digest fields,
  and `retryable: false`. The text message also includes the written tip.
  Inspect the current handoff before taking another action: verification is
  incomplete and a concurrent writer may already have advanced the ledger.
- Approval policy: `assign` accepts `--approval-policy <policy>` with values
  `source-pass` and `final-approval`. New assignments default to `source-pass`,
  preserving documented COMMENT-only source closure. Omission during handover
  or refresh preserves the existing policy. Missing policy fields in old
  records also mean `source-pass`; serialization omits that default so existing
  record digests and ledgers remain valid. Changing policy requires ordinary
  surrender/recovery and a fresh assignment interval; a base-only scope refresh
  cannot change it.

  Assigned delivery and merge enforce the persisted policy, including explicit
  merge review opt-out. `final-approval` requires a canonical passing native
  `APPROVED` report on the current provider head. `check` accepts the same flag
  to demand stricter approval for that invocation; it cannot weaken the
  assignment. A source pass under a strict policy returns DATA 65,
  `designated_final_approval_required`. No natural-language hold phrases are
  interpreted. The newest admissible report still owns the verdict; an earlier
  approval cannot override a newer source-only or blocked report.
- `inspect <id> [--expected-head <sha>] [--review-author <login>]` validates the
  supplied head and any existing assignment before private mailbox handoff.
  Base drift returns success with `status: review-scope-changed`, the current
  provider `base_sha`, and the unchanged persisted `handoff`, `handoff_digest`,
  and exact durable `state_tip_digest`. This exposes coordinator/generation
  details for recovery without admitting stale review evidence. Explicit head,
  reviewer, and author mismatches still fail closed.
  An unassigned result is bootstrap discovery, not admission; append and verify
  the assignment before sending private evidence.
  `check <id> --expected-head <sha>` requires a reviewer-owned closed observation
  after handover and a canonical native report bound to the same PR, base,
  head, and appointed author. GitHub login comparison is case-insensitive.
  Reviews are considered newest-first. A report is skipped when its single
  canonical binding identifies a different handoff retained in the verified
  chain. An empty `COMMENTED` review from the appointed author on the current
  head is also skipped after complete native body/actor readback and a bounded
  review-comment read confirms only thread replies, or no comments. Root inline
  comments retain normal report validation. Missing or mismatched provenance,
  invalid data, or a full 100-comment page returns `review_snapshot_incomplete`
  rather than treating a partial result as empty. The newest remaining appointed-author review must be
  `COMMENTED` or `APPROVED`, have a `pass` or `follow-up-pass` verdict, and
  postdate handover. A report in the same timestamp second fails closed. Long
  summaries use bounded native-body read-back with verified identity and head.
  A newer current-interval blocked or malformed report supersedes an earlier
  pass. Unbound later-generation reports and ambiguous, malformed or unknown
  bindings are not skipped.
  Admission applies the same exact Reviewable reference binding as publication.
  An optional full-SHA qualifier must match the native review's current head.
  A rejected report returns `awaiting_designated_review` with detail
  `review_id=<id>; field=Reviewable; cause=<validation message>`.
  Live designated `pr review --submit-review` binds both summary-only and
  threaded native reports to the admitted handoff record with the opaque marker
  `<!-- forge-cli:review-handoff:v1 <handoff-digest> -->`. This binding is captured
  before publication, so an old-generation report that lands after a handover
  cannot authorize the new interval even if its timestamp is newer. Generation
  two and later require exactly one matching binding; existing first-generation
  reports without a marker remain compatible. The selected report's present
  marker must match the current handoff in every generation. Reassigned reviewers must publish through
  a CLI that supplies the binding; timestamps alone no longer admit those reports.
- Base-sync carry-over: when the provider head H' is not the reviewed head H
  of the owned observation, `check` admits the existing designated PASS only
  if all of these hold:
  - H' is a commit with exactly two parents, H first and base commit B second.
  - The provider base branch name equals the `base_ref` that `assign` recorded
    from the pull request view. A handoff without `base_ref` never carries, so
    a retarget never carries, even to a branch descending from the assigned
    base.
  - The assigned base and B are both ancestors of the current provider base.
    The base tip and branch name come from one provider read.
  - `git merge-tree --write-tree` of H and B is conflict-free, and its tree
    equals H'^{tree}.

  Every git fact is evaluated in a throwaway bare repository that borrows only
  the caller's object store. The caller's attributes (including merge drivers
  in the checkout or `info/attributes`), config, refs, replacement refs and
  hooks never shape the result.

  Any other case keeps the re-review refusal `awaiting_designated_review` with
  detail `sync_carry_over=<reason>`. That includes a conflicted sync, a later
  commit, a forged tree, a base parent off the target branch, a changed or
  unrecorded base branch, and objects missing locally. Hard stops, open
  findings, author binding, interval binding, and newest-report ownership still
  apply, so a newer report on H' supersedes the PASS on H.

  A live `check` appends a `review-sync-carry-over` record once (`review_url`,
  `reviewed_head`, `head`, `base_commit`, `merge_tree`) and reports it as
  `sync_carry_over`; `--dry-run` reports it without writing. Any session with
  an active assignment may record it through `check`, including the
  coordinator. The record is never trusted alone, because every admission
  recomputes it. Delivery and merge gates re-verify the carry-over and admit
  it only when that exact record is in the active interval; otherwise they
  refuse and point to `check`.

  For a carried merge, the merge payload's `review_loop.head_sha` is the
  reviewed head H. `inspect` reports the recorded carry-over for the current
  head as-is, without re-verifying it, and its `status` stays derived from
  the handoff (for example `review-scope-changed` after the base moved).
  Provider-side reviews are never changed.

  Forge-cli releases before this change fail closed when they read a ledger
  that contains the new record kind or a handoff `base_ref`. The fleet must
  run this release before the first live carry-over or assignment writes one.
- `surrender <id> --expected-head <sha> --expected-state <digest>` is a
  reviewer-owned relinquishment at the current assignment generation. The
  exact retained ledger tip selects that generation, so surrender does not
  require `AGENT_REVIEW_ASSIGNMENT_GENERATION` in the reviewer environment.
  Review observations and publication still require that variable, using the
  generation returned by `inspect`. Surrender does not publish a review or
  authorize a verdict. A wrong public-author assignment can therefore be
  surrendered by its persisted reviewer and reassigned at the same PR head.

  Ordinary reassignment requires this explicit surrender; inspection alone
  never transfers ownership. Surrender may name the current provider head
  whether or not an observation exists, but after an advance it may name the
  assigned head only when that head belongs to the current assignment and no
  observation exists in that interval; prior-assignment observations do not
  block it, and a current-assignment observation requires naming the current
  head. The surrendered record stores the provider-head snapshot read before
  append without rechecking it there, so a second advance during append leaves
  the earlier snapshot recorded; the ledger-tip compare-and-swap still guards
  the state chain, and a later assignment must match the then-current head.
- `recover <id> --expected-head <sha> --expected-state <digest> --reason
  <reviewer-unreachable|reviewer-closed|reviewer-declined|reviewer-timeout>` is
  an explicit original-coordinator revocation that advances the assignment
  generation. The parent owns mailbox transport, bounded waits, and liveness
  evidence; the CLI never guesses reviewer availability. Surrendered or revoked
  control fails with `designated_reviewer_unavailable`, never self-review.
  Recovery captures the current provider head/base even after base drift.
  The current recorded coordinator may assign the next generation using the
  durable `state_tip_digest` returned by recovery or fresh inspection.

  For an incorrect assignment, the original coordinator can use
  `--reason reviewer-declined`; this does not require supervisor configuration
  or `--reason coordinator-retired`. Assignment-time validation of a remote
  reviewer session's publishing author remains unavailable: the identity
  policy authenticates the calling session, not an arbitrary remote reviewer.
- Successor takeover uses `recover <id> --reason coordinator-retired
  [--coordinator-session <recorded-session-id>[@machine]] --base-sha <current-base>
  --expected-head <current-head> --expected-state <exact-tip>`. The selector
  when supplied must hash to the recorded coordinator; otherwise the selector
  is resolved from the stored coordinator digest in a fresh, complete
  `agent-session board --state all --format json` read. Only a repository
  configured handoff supervisor may take over, as listed by `sha256:` digest in
  `[review_handoff].supervisors` at the exact provider base. The selected
  remote must match provider host/repository and its base-tracking ref must
  resolve to that base SHA; invalid explicit `stale_after` refuses takeover.
  The list is
  empty by default and is never read from global configuration or the PR head.
  The supervisor must also prove at operation time that every matching
  coordinator row is stopped or deleted on one available host and that its
  `last_terminal_activity_at` is older than `[review_handoff].stale_after`
  (default `2h`). A live coordinator is refused with
  `review_coordinator_live`; absent, recent, unknown, vanished, truncated,
  unavailable, malformed, or failed evidence is refused with
  `review_coordinator_unproven`. The supervisor identity, reason, host digest,
  terminal status, activity time, and prior coordinator digest are retained in
  the new ledger generation. Registry absence alone grants no ownership.
  The opaque source digest binds the final proof read without publishing its
  machine name. Source configuration remains owned by `agent-session`;
  `FORGE_CLI_AGENT_SESSION_BIN` selects its executable (including test adapters).
- A `review_coordinator_unproven` refusal carries `details.prerequisite`,
  `details.retryable` and `details.next_action`; its message states the remedy.
  Policy prerequisites: `checkout_unavailable`, `remote_unavailable`,
  `remote_repository_mismatch` (pass `--remote`), `base_ref_missing` and
  `base_ref_stale` (`git fetch <remote> <base>`), `provider_base_behind_cache`
  (the provider base predates the cached branch tip; update the pull request
  branch onto the base), and `policy_config_invalid`. An empty supervisor list
  is a `review_writer_conflict` that names the missing `[review_handoff].supervisors`.
- Lifecycle prerequisites: `session_board_unavailable`, `session_board_incomplete`,
  `coordinator_not_on_board`, `coordinator_source_ambiguous`,
  `coordinator_source_selector_mismatch` (correct or omit the `@machine`
  selector), `lifecycle_source_unavailable`, `coordinator_not_terminal`,
  `terminal_activity_missing` and `coordinator_not_stale`. A retained row
  without `last_terminal_activity_at` is never repaired from other timestamps or
  caller input; only the session-board operator can supply that evidence.
  A coordinator deleted or archived by `agent-session` is retained with its
  close time as `last_terminal_activity_at`, so a configured supervisor can
  take over once `stale_after` has elapsed; rows closed by an older release
  may still lack it.
- Safe coordinator rotation: hand over every review handoff (`recover` by the
  original coordinator, or reassign) before deleting the old coordinator
  session. Takeover by a supervisor is the fallback for a coordinator that was
  retired first.
- Takeover appends a handoff recovery with `coordinator_transfer` containing
  `old_coordinator_digest`, `new_coordinator_digest`, `lifecycle_source_digest`,
  `reason` (`coordinator-retired`), `supervisor_digest`, and
  `retirement_evidence` (`machine_digest`, `session_status`,
  `last_activity_at`, and `stale_after_seconds`). This optional field is omitted on ordinary records,
  preserving their existing digest bytes. The assignment generation advances,
  the current head/base are captured, reviewer identity and public author stay
  fixed, and every inherited finding/history record remains attached. Takeover
  revokes the old review interval; it does not authorize a verdict. The
  successor may then `assign` normally; the reviewer must append a fresh owned
  observation. Coordinator publication remains prohibited.
- Takeover requires exact provider head/base and the caller's retained ledger
  tip, including `--dry-run`. Before a live append, the CLI rereads tip,
  ownership, session lifecycle, and provider head/base. A recovery racing an
  old-generation reviewer write follows the existing recovery-wins rule;
  competing takeovers fail closed. Dry-run emits prospective transfer metadata
  without a provider write or refreshed durable tip.
- Assignment mutations require the caller's retained exact ledger tip; they
  never refresh it to hide a competing writer. Every operation emits
  `cli.forge-cli.pr.review-handoff.v1` with `number`, `url`, `head_sha`,
  `base_sha`, `state_tip_digest`, `handoff_digest`, `handoff`, and `status`.
  Nonempty `ignored_stale_records` reports records superseded by recovery.
  `--dry-run` performs read-only admission and reports prospective ownership
  while retaining the observed durable tip.
  Before assignment or recovery appends, tip/ownership and provider head/base
  are reread. Surrender can retain a stale base so the reviewer can relinquish
  the old interval after integration.
- Approval output: every handoff operation also emits effective
  `approval_policy` (`source-pass` or `final-approval`). Successful `check` and
  publication-admitted `inspect`
  retain `status: reviewed` for compatibility and add `approval`: `state`
  is `source-passed` or `finally-approved`; `review_id`, `review_url`,
  `review_author`, `native_state`, and `reviewed_head` identify the selected
  native report. `finally-approved` requires `APPROVED` on the current head.
  Approval carried from an older head across a verified base sync remains
  source closure and cannot satisfy `final-approval`.

  Active unchanged-base `inspect` uses the same publication admission and
  persisted policy as `check`, exposing `status: reviewed` and `approval` even
  when the owned observation has no findings. Missing or inadmissible reports
  and unmet policy retain `awaiting-designated-review` without `approval`;
  provider and incomplete snapshot errors remain failures. Inspection never
  appends a ledger record. Other operations omit `approval`.
  Consumers requiring final approval must select the strict policy or inspect
  `approval.state`; existing `status: reviewed` alone does not mean final
  delivery approval. Independent delivery checks still apply.

`AGENT_SESSION_ID` fences the invoking session; routing metadata does not grant
provider authorization. Reviewer appends and native publication also require
`AGENT_REVIEW_ASSIGNMENT_GENERATION` equal to the retained assignment generation.
The final append rereads ownership, generation, tip, and head. If recovery or a
same-reviewer scope refresh races an old-generation write and both reach the
provider, the unique higher-generation interval wins; stale children and
descendants are ignored and reported, never
silently merged. Other competing children still fail closed. A repair before
the first designated observation is rejected as `review_repair_unobserved`.
The first owned observation appends even at an unchanged head and preserves
inherited findings and history. Reassignment requires a fresh owned observation.

Live assigned `pr deliver`, including `--no-merge`, returns
`awaiting_designated_review` after creating/adopting the PR and checking CI,
before ready/merge. Its failure envelope retains PR URL/number. A delivery
`--dry-run` is a local command preview; use `review-handoff check` for readiness
proof. Assigned merge preview and live merge use the same gate. Live delivery,
merge, and publication detect persisted assignment with the environment absent.
Mailbox `pass`, stale publication, missing handoff, open findings, and unavailable
reviewers cannot satisfy admission. Existing convergence and merge gates remain.

Designated publication admission additionally enforces
[test-change declarations and independent reviewer confirmation](test-change-admission-v1.md)
for deleted tests, assertion changes and new skips in the PR comparison.

### `pr review-loop inspect` / `observe` / `extend` / `validate`

- These GitHub-only commands make the repair/re-review loop resumable from the
  append-only `forge-cli.review-loop.v1` chain. `inspect <id>` validates the
  complete chain and emits its typed latest state. `observe <id>
  --expected-head <sha> --findings-file <path> [--expected-state <digest>]
  [--body <text> | --body-file <path>]`
  accepts either a delivery-mode `review-specialists merge` envelope or a
  finding-observation array, evaluates one deterministic transition, and uses
  both the exact PR head and state tip as compare-and-swap inputs.
- `observe --preflight` runs the non-short-circuiting verdict sweep `--dry-run`
  reports, in front of the live append, and stops with `review_preflight_failed`
  naming every failing rule rather than the first. It exists so one call can do
  what a caller otherwise writes as a `--dry-run` call, a `preflight_ok`
  assertion, and a live call; the provider reads are the same ones that sequence
  already paid for. Under `--dry-run` it adds nothing, because the dry run is
  the sweep. `--preflight` with `--body-file -` is refused as
  `review_preflight_body_not_replayable` before the first provider call: the
  sweep and the append each read the body once, so a piped body would be
  validated in one and submitted in the other.
- `observe --auto-state` reads the current chain tip and compare-and-swaps
  against it, instead of being handed one with `--expected-state` (the two
  conflict). It is deliberately the weaker of the two, and the difference is not
  ergonomic: `--expected-state` is a claim about a tip the caller *already saw*,
  which is what catches a resumed shell reusing genesis state, and no
  self-read can reconstruct that claim. Use `--auto-state` for the genesis
  observation, where the tip is read fresh anyway and there is nothing earlier to
  assert; keep `--expected-state` for a closing observation made against the
  digest an earlier append returned. With `--preflight` the tip is read twice,
  and a move in between fails as `review_state_tip_moved` naming both digests —
  never a silent re-read, because the transition was computed against the chain
  the sweep read and a moved tip makes that computation stale.
- `observe --body <text>` / `--body-file <path>` (`-` reads stdin; mutually
  exclusive) posts a human-readable delivery outcome in the SAME provider comment
  as the appended ledger record, replacing a separate final outcome comment.
  Every rule on the body runs before the first provider call, so a dry run
  returns the same verdict a live run would and a rejected body costs no provider
  round trip. The body must be non-empty, within the comment-body limit
  (`review_state_comment_too_large`), free of any HTML comment
  (`review_state_comment_invalid` — a review-state marker of its own would shadow
  this record because the parser takes the first marker in a body, and an
  unterminated `<!--` would hide the visible label), and held to the same
  portability rules as a review comment (`local_path_present`,
  `markdown_escaped_control`). Those portability rules are the shared
  provider-payload guard, which is pattern-based and allowlisted; the outcome body
  is operator-attested text, not a machine-derived receipt field, so the stronger
  "review state excludes local paths" clause above is not a guarantee about it.
  `--body-file -` consumes stdin once, so a dry-run-then-live sequence must not
  share a piped body.
- Because the outcome rides the append, it inherits the append's idempotency: an
  unchanged observation appends nothing, so a supplied outcome is either already
  present on the pull request from the earlier attempt — reported as
  `outcome_posted: true` with `appended: false` — or would be silently dropped,
  which fails closed as `review_outcome_not_posted` instead. An identical retry
  therefore cannot create a duplicate outcome or ledger comment, and cannot lose
  one either. `outcome_posted` is never inferred from the flag: it is set only
  after a post-write read-back confirms a privileged comment carrying both this
  record's marker and this outcome, and a durable record whose outcome is not
  visible fails as `review_outcome_not_posted`. A durable hard-stop receipt is
  always appended without an outcome body.
- Applicability is narrow and deliberate. The combined form fits a caller whose
  delivery outcome is decided once and never revised, because a ledger record is
  immutable and an append is conditional: an unchanged observation has no append
  to carry a revised outcome. It does NOT fit a workflow whose outcome must be
  posted after repairs and refreshed on retry — there the outcome is a mutable
  artifact and the ledger is not, so welding them either fails closed, appends a
  semantically empty generation per refresh, or leaves a stale outcome beside a
  newer one. `agent-runtime-kit`'s delivery skills are in that second category:
  their posting-order contract requires the disposition to post last and to be
  refreshed on merge-convergence retries, so they keep the outcome as its own
  comment by design. They still benefit from the visible checkpoint sentence above,
  which is what stops a ledger comment from rendering blank. Do not adopt the
  combined form in a workflow without first checking that its outcome is
  single-shot.
- Observation-array rows may include `status` or `disposition` with `open`,
  `fixed`, `accepted`, `preference`, or `follow-up`. Terminal dispositions are
  durable and non-blocking. Every new-head observation must include each
  previously open identity; omissions fail as `review_observation_incomplete`
  with all missing identities and leave the ledger unchanged. Changing an open
  finding's blocking bit on the same head still fails closed.
- Ordinary observations support delta input: omitted terminal findings retain
  their recorded dispositions and lifecycle metadata. Every previously open
  finding must be included, on both unchanged and new heads. Full-set input may
  reassert an already `fixed` finding at the same head without advancing its
  lifecycle counters; thread references are merged and deduplicated. For example,
  after F1 is fixed at head H, either `[F2 open]` or `[F1 fixed, F2 open]` at H
  retains F1 and admits F2. An identical full-set retry appends nothing. First
  marking an open finding `fixed` still requires a repaired head; the error
  explains that already fixed findings can instead be omitted or repeated.
- To repair a historical omission error, pass an explicit correction envelope
  through the existing `observe --findings-file` path and an exact
  `--expected-state` tip. `--auto-state` cannot admit a correction.

  ```json
  {
    "schema_version": "forge-cli.review-loop-correction.v1",
    "correction": {
      "corrected_record": "sha256:<erroneous-record-digest>",
      "reason": "omitted-findings"
    },
    "findings": [
      {"lifecycle_fingerprint": "correctness:component:invariant", "status": "open"}
    ]
  }
  ```

  Include the complete current inventory using its canonical identities,
  including unchanged terminal findings.
  The named ordinary observation must be at the current head. The chain must
  prove each restoration was an open finding whose status alone became fixed:
  a genuinely observed fixed disposition cannot be restored through this path.
  Unrelated dispositions, blocking bits, roots, and thread metadata cannot change.

  A `review-loop-correction` record appends the restored state and a receipt with
  `corrected_record`, `reason`, and sorted `restored_findings`. Existing record
  bytes and digests stay intact. Lifecycle/repair/reopen counters, budgets, and
  extensions are retained. Lost no-progress debt is restored from the proven
  prior inventory, including a durable stop if its budget is exhausted.
  A matching unextended false-reopen stop
  may be cleared only for the identical attempted inventory; unrelated or already
  extended stops require their normal path.

  If the original attempted rows used a lifecycle alias distinct from its root,
  include those rows in optional `correction.attempted_findings`. They accept
  the ordinary observation keys, including `lifecycle_fingerprint` and
  `disposition`. Their digest must exactly match the stored stop, while their
  canonical inventory must match the corrected dispositions, blocking bits,
  roots, and threads. Proof without a false-reopen stop is refused.

  The receipt retains normalized, sorted `attempted_findings` when supplied.
  Omission of the optional proof leaves older receipt bytes unchanged.
  Chain parsing replays the proof and rejects altered aliases, inventories,
  budgets, or unrelated findings. Use a CLI version that understands this
  record kind to read a corrected chain.

  An identical retry against the fresh corrected tip appends nothing within
  its original assignment. Reusing that receipt after reassignment fails as
  `review_correction_invalid` before any provider write. Establish the new
  reviewer interval with an ordinary observation inventory instead.
- The default budget is five repair rounds, two consecutive no-progress rounds,
  and zero automatic reopens for a previously fixed lifecycle fingerprint.
  Same-head/same-findings retries append nothing. Only a new reviewed head
  advances a round. The no-progress budget applies only while open blocking
  findings remain; a clean reviewed head resets it. A strict reduction in
  open-finding cardinality is mechanical progress. Stable-fingerprint collisions, reopens, no progress,
  and the round limit fail closed as typed terminal errors. An extendable error
  first appends a durable hard-stop receipt, so a restart returns the same stop.
- A durable terminal budget error includes an exact extension proposal, but
  never extends itself. `extend` is a separate action requiring the stopped state tip,
  expected head, proposal digest, field/increment, stop code, and a newer
  comment on the same PR from an `OWNER`, `MEMBER`, or `COLLABORATOR` carrying
  the exact approval marker. The stop code must map to its canonical budget
  field and the proposal must match the active receipt. Each proposal is
  consumable once; retry and delivery paths cannot silently increase a budget.
- To terminally accept remaining risk after a durable hard stop, use an
  explicit acceptance envelope with `observe --expected-head <stopped-head>
  --expected-state <current-tip> --findings-file acceptance.json`:

  ```json
  {
    "schema_version": "forge-cli.review-loop-acceptance.v1",
    "acceptance": {
      "stopped_record": "sha256:<latest-stopped-review-loop-record>",
      "attempted_findings": [
        {"fingerprint": "correctness:component:invariant", "status": "open"}
      ],
      "approval_comment": 123
    },
    "findings": [
      {"fingerprint": "correctness:component:invariant", "status": "accepted"}
    ]
  }
  ```

  `attempted_findings` must prove the exact canonical inventory digest in the
  stop, including thread references and root-cause identities. The proposed
  `findings` may only change `open` rows to `accepted`; every other row must
  remain identical, every previously open identity must be present, and no
  blocking open finding may remain. `accepted` represents accepted risk or a
  decision not to fix. It does not claim that a repair occurred.

  First omit `approval_comment` and run `observe --dry-run` to obtain the exact
  required marker in the `observation_transition` verdict. The proposal digest
  binds the stopped record, head, attempted inventory, and proposed inventory.
  A maintainer then posts that marker as a standalone line in a newer comment
  on the same PR, authored by an `OWNER`, `MEMBER`, or `COLLABORATOR`:

  ```text
  <!-- forge-cli:review-loop-acceptance:v1 proposal=sha256:<proposal-digest> -->
  ```

  Set `approval_comment` to that comment's numeric ID, rerun `--dry-run`, then
  perform the live observation against the explicit current tip. An extension
  approval marker grants no acceptance authority. Missing, unauthorized, stale,
  wrong-PR, or mismatched approval fails as `review_acceptance_approval_invalid`
  before any write. Invalid proof, altered dispositions, a stale stopped record,
  or `--auto-state` fails closed; inventory violations use
  `review_acceptance_invalid`. Both acceptance errors expose typed recovery
  details (`retryable`, `next_action`, and `recovery`). Existing head, tip,
  designated writer, native review, and merge gates continue to apply.

  The appended `review-loop-acceptance` record retains the original stop and
  canonical attempted inventory, named accepted fingerprints, proposal digest,
  and approval reference. Original records remain byte-identical. The latest
  state clears the active stop and records the dispositions at the stopped
  head, preserving rounds, no-progress counters, budgets, extensions, and
  existing finding lifecycle counters. No extension or new repair round is
  required, including when an extension was already applied. Convergence then
  passes when the other merge gates pass. `inspect` and `observe` expose the
  historical receipts in `accepted_risks`, including the original stop.
  Identical retries at the fresh tip append nothing; a retry from an older
  designated assignment cannot establish a new review interval. Readers must
  understand the new record kind before consuming a chain that contains it.
- An applied extension still binds the stopped head and the exact canonical
  inventory: reviewer surrender/reassignment, a new assignment generation, or
  a changed base does not clear that binding. A mismatched observation returns
  `review_state_conflict` with recovery commands in the message, including in
  `observe --dry-run` preflight. Start with
  `forge-cli pr review-loop inspect <PR> --format json`.

  If the provider head is unchanged, replay the original inventory whose digest
  matches `hard_stop.observation_digest`:

  ```bash
  forge-cli pr review-loop observe <PR> --expected-head <stopped-head> \
    --expected-state <current-tip> --findings-file <original-findings.json>
  ```

  The already applied extension is not applied again. An unapplied extension
  still requires its exact approval and `extend` before replay. An altered
  inventory cannot reuse that approval. If the provider head advanced, follow
  the signed fast-forward supersession below instead; it requires fresh
  approval even when the previous extension was consumed. Unsupported head
  changes remain blocked. Neither recovery resets history or grants budget
  implicitly.
- When a stopped attempt at A is superseded by a signed fast-forward to B,
  inspect the current tip and pass this explicit findings-file wrapper to
  `observe` with `--expected-head B --expected-state <current-tip>`:

  ```json
  {
    "schema_version": "forge-cli.review-loop-supersession.v1",
    "supersession": {
      "superseded_record": "sha256:<latest-stopped-state-record-digest>",
      "reason": "signed-fast-forward"
    },
    "findings": [
      {"lifecycle_fingerprint": "correctness:component:invariant", "status": "open"}
    ]
  }
  ```

  Include every open finding. The provider must prove A is an ancestor of B
  and every newly reachable commit has a valid verified signature. Comparison
  proof is read completely, with a bound of 100 commits; divergence, unsigned
  commits, truncated proof, stale tips, and unknown fields fail as
  `review_supersession_invalid`. Offline `validate` checks only the input shape;
  `observe --dry-run` also checks provider proof, writer ownership, head and tip.
  The independent 64 KiB marker and complete-comment limits remain binding;
  large retained inventories may reach those limits below the commit cap.

  A `review-loop-supersession` record retains the stopped A attempt and all prior
  records. Its receipt names the superseded record, A, verified commit IDs, and
  canonical findings. Readback replays the transition and rejects counter or
  budget mutation. The successful observation head stays unchanged until resume;
  the enclosing record and fresh stop bind B. Counters, findings, budgets, and
  consumed extensions remain intact. Even if A's extension was already consumed,
  B gets a fresh proposal with `extension_applied: false`.

  Inspect that new tip,
  obtain a newer maintainer approval carrying B's exact proposal marker, run
  `extend` at B, then resume the identical B inventory through ordinary `observe`.
  A's approval cannot be reused. An identical retry at the fresh tip appends
  nothing within its original reviewer assignment. After reassignment, the old
  receipt cannot bootstrap the new interval and is refused before provider writes.
  Use the current stop's approval/extension path before an ordinary observation.
  No force push or manual ledger edit is needed. Older readers must upgrade before
  reading the new record kind; designated reviewer rebinding remains required
  when the assigned head changes.
- An observation can only be appended at the current provider head, so history
  cannot be backfilled, and first marking a finding `fixed` requires a repaired head.
  Together those force the observation to precede the repair push: observe the
  reviewed head as `open`, push the repair, observe the repaired head as
  `fixed`, then merge at that head. Doing every repair first and then trying to
  record the history is unrecoverable, because the pre-repair head is gone and
  the round count cannot be reconstructed.
- The three provider-backed commands emit their own
  `cli.forge-cli.pr.review-loop.*.v1` schema with `data.appended` and
  `data.outcome_posted`. Only `observe` can set `outcome_posted`, and never
  without `appended`. `inspect` and `extend` dry-runs are offline and report the
  command-specific read/transition plan.
- `validate --findings-file <path>` is the offline payload check, mirroring
  `pr review validate`. It emits `cli.forge-cli.pr.review-loop.validate.v1` with
  `data = { shape, row_count, identity_count, dispositions[], blocking_count,
  duplicate_identities? }` and carries neither `appended` nor `outcome_posted`,
  because it never appends. It takes no id, resolves no provider context and
  runs no backend, so it is provider-independent and `--provider`, `--repo`,
  `--host` and `--dry-run` have no effect on it.

  Its accept/reject decision is not a second implementation of the payload
  rules: it calls the same canonicalization `observe` calls, so lifecycle
  fingerprint form, identity collisions (`review_fingerprint_collision`) and
  blocking normalization for terminal dispositions are decided by one piece of
  code. Head and state-tip compare-and-swap, the transition against stored
  state, and the rendered comment need the provider and stay with
  `observe --dry-run`. Failing `validate` therefore proves an append cannot
  succeed; passing means the payload itself is acceptable.

  Accepted dispositions are `open`, `fixed`, `accepted`, `preference` and
  `follow-up`. A finding that reappears is submitted as `open`; the state
  machine decides whether that is a reopen, and `reopened` is not an input.
- `observe --dry-run` is a faithful non-mutating preflight. It reads and
  validates the findings payload and any outcome body, resolves the pull request,
  performs the head
  and state-tip compare-and-swap comparisons, evaluates the transition, and
  renders the exact comment the live append would post, then reports each
  verdict in `data.preflight[]` — the same element shape as
  `pr deliver --dry-run`'s `local_preflight[]`, under a different name because
  these rules include provider reads. `data.preflight_ok` is the conjunction and
  `data.would_append` reports whether the real run would append a new
  generation — true for an accepted state-changing transition, and also for an
  extendable budget error, which appends a durable hard-stop receipt before
  failing, so predicting only "this fails" would understate it. When an
  unextended durable hard stop already exists the live path returns that stop
  *without* appending, and the dry run mirrors that: `would_append` is false and
  no comment is planned.
  `data.planned_comment` reports the planned write as
  `{visible_metadata, includes_outcome_body, bytes}`, where the established
  `visible_metadata` field now carries the checkpoint sentence and `bytes` is the
  complete rendered body the size limit binds. It is present only when a write is
  planned, and the `state_comment_body` verdict is likewise reported only then: an
  already-current chain writes nothing, so there is no body to check and no
  outcome to post. The sweep does
  not short-circuit, so the local payload verdict is
  reported even when the provider is unreachable; that is the supported way to
  check a findings file without writing durable provider-visible state, which a
  live `observe` does on success.

### `pr pending-review delete` compatibility surface

- `pr pending-review delete <id> --review <PRR_...> --expected-head <sha>
  --expected-commit <sha> (--expected-body <text> | --expected-body-file <path>)
  --confirm-abandoned` emits
  `cli.forge-cli.pr.pending-review.delete.v1` and is a GitHub-only recovery
  primitive. GitLab and local return `provider_unsupported` (`USAGE 64`).
- Before mutation it fetches the named PR, reads a complete paginated
  pending-only membership snapshot while retaining only the named review body,
  and requires `--review` to name an entry whose PR head, review commit, and
  normalized body exactly match the required expected values. The entry must
  also have provider-native
  `viewerDidAuthor: true` and `viewerCanDelete: true`. A missing or
  already-submitted node returns `pending_review_not_found`; ownership and
  delete-capability failures return `pending_review_author_mismatch` and
  `pending_review_not_deletable`; content drift returns the corresponding
  `pending_review_*_mismatch`. Submitted-review parsing is independent and
  cannot block this recovery path.
- After the membership checks, the command takes the private cross-process
  viewer/repository/PR lease and reads the exact review node again immediately
  before deletion. It revalidates PR membership, head, commit,
  body, ownership, and delete capability. `pending_review_pr_mismatch` rejects
  a moved or inconsistent target. Reviews with inline draft comments return
  `pending_review_inline_comments_present` and require manual provider recovery;
  this primitive deletes only confirmed abandoned body-only drafts.
- Only the revalidated node id is passed to `deletePullRequestReview`, the
  returned id must match, and an exact-node read must reconcile provider absence
  before success. The success payload is
  `data = { provider, number, url, head_sha, commit_sha, review_id, review_url,
  author, deleted }`. GitHub does not expose content compare-and-swap on this
  deletion mutation. The lease excludes cooperating forge-cli processes, while
  explicit abandonment confirmation, immutable content guards, reconciliation,
  and the inline-comment refusal bound the irreducible same-identity
  non-cooperating-client window between the exact final read and delete.
- `--dry-run` is offline. It validates a named `--expected-body-file`, renders
  the PR guard, complete snapshot, exact-target read, and delete plans, and
  never emits the expected body or its file path.
- Expected and provider review bodies are limited to 64 KiB. Inline values and
  named files are checked before provider access; file and live stdin inputs use
  bounded reads. Oversized expected input returns
  `pending_review_body_too_large`, while an oversized provider body fails the
  snapshot closed as `review_snapshot_incomplete`.
