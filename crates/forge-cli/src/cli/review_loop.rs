use super::*;

#[derive(Args, Debug, Clone)]
#[command(after_help = "APPEND ORDER\n  \
      An observation can only be appended at the CURRENT provider head, so \
      history cannot be backfilled, and first marking a finding `fixed` requires a \
      repaired head. Observe the reviewed head BEFORE pushing the repair:\n    \
      observe --expected-head <reviewed head>   (disposition: open)\n    \
      repair, push\n    \
      observe --expected-head <repaired head>   (disposition: fixed)\n    \
      merge --expected-head <repaired head>\n\n\
      --findings-file SHAPES\n  \
      1. A `review-specialists merge --mode delivery` envelope. Each row needs \
      `evidence`, `recommendation`, and a `lifecycle_fingerprint` of the form \
      `<category>:<component>:<invariant>` whose category segment equals the \
      row's `category`. The envelope schema REJECTS `disposition` as an unknown \
      field.\n  \
      2. A bare observation array. Each row needs `lifecycle_fingerprint` and \
      accepts `disposition` (open | fixed | accepted | preference | follow-up), \
      which is how dispositions are carried across rounds. A finding that \
      reappears is submitted as `open`; the state machine decides whether that \
      is a reopen, and `reopened` is NOT an accepted input.\n  \
      3. A `forge-cli.review-loop-correction.v1` object with `correction`\n  \
      (corrected_record digest, reason omitted-findings) and complete `findings`.\n  \
      Use explicit --expected-state; only chain-proven omissions can be restored.\n  \
      Old history and budgets remain unchanged. New heads require all open findings.\n  \
      An alias-bearing false-reopen stop requires correction.attempted_findings\n  \
      containing the original attempted rows; its digest must match exactly.\n  \
      After reassignment, observe ordinary inventory before retrying a correction.\n  \
      4. A `forge-cli.review-loop-supersession.v1` object with `supersession`\n  \
      (superseded_record digest, reason signed-fast-forward) and current `findings`.\n  \
      Use explicit --expected-state. A verified signed fast-forward records a new\n  \
      stop requiring fresh maintainer approval at the new head; history stays intact.\n  \
      Proof is capped at 100 commits; complete record/comment size limits still apply.\n  \
      Earlier-assignment retries fail before writes; use approval/extension to resume.\n  \
      OBSERVATION INVENTORY\n  \
      Delta input may omit terminal findings: their recorded dispositions are \
      retained. Include every previously open finding, even on the same head.\n  \
      Full-set input may repeat an already fixed finding on the same head; \
      its lifecycle counters remain unchanged and thread references are merged.\n  \
      Example: after F1 is fixed at H, submit F2 open alone or F1 fixed plus \
      F2 open at H. First marking an open finding fixed still needs a new head.\n  \
      Check these shapes offline, with no pull request and no provider call, \
      using `pr review-loop validate --findings-file <path>`.\n\n\
      COMBINED DELIVERY OUTCOME\n  \
      Every appended ledger comment leads with a record-specific plain-English \
      checkpoint sentence above its machine marker, so it never renders as a \
      blank comment.\n  \
      --body / --body-file additionally posts a human-readable delivery outcome \
      in that SAME comment, after the marker, replacing a separate final outcome \
      comment. An outcome body may not contain an HTML comment, and is checked \
      before the first provider call.\n  \
      The outcome rides the append, so it inherits the append's idempotency. When \
      the observation changes nothing the ledger is not appended, so the outcome \
      is either ALREADY present from the earlier attempt (data.outcome_posted \
      true, data.appended false) or would be dropped, which fails closed as \
      `review_outcome_not_posted`. An identical retry therefore cannot duplicate \
      the outcome, and cannot lose it either. data.outcome_posted is confirmed by \
      a post-write read-back, never assumed from the flag. A durable hard-stop \
      receipt is always appended without an outcome body.\n  \
      --body-file - consumes stdin once, so do not pipe the same body into a \
      --dry-run and then a live run.\n  \
      APPLICABILITY: use the combined form only when the outcome is decided once \
      and never revised. A ledger record is immutable and an append is \
      conditional, so a workflow that must post its outcome after repairs and \
      REFRESH it on retry has no append to carry the revision and should keep the \
      outcome as its own comment — the visible checkpoint sentence above already stops \
      ledger comments from rendering blank.\n\n\
      With --dry-run this runs a faithful non-mutating preflight: it reads and \
      validates the findings payload and any outcome body, resolves the pull \
      request, performs the head and state-tip CAS comparisons, evaluates the \
      transition, and renders the exact provider comment it would post, then \
      reports every verdict in data.preflight[] without appending. The sweep \
      does not short-circuit, so the payload verdict is still reported when the \
      provider is unreachable — use it to check a findings file without writing \
      durable provider-visible state.\n\n\
      --auto-state AND --preflight\n  \
      --preflight runs that same sweep in front of a LIVE append and stops with \
      `review_preflight_failed`, naming every failing rule, instead of the first \
      one. It replaces the separate --dry-run call a caller would otherwise make, \
      and costs the same provider reads that call did.\n  \
      --auto-state reads the current chain tip and compare-and-swaps against it, \
      so a caller does not have to `inspect` and thread the digest through. It \
      conflicts with --expected-state.\n  \
      What --auto-state does NOT detect: a tip that moved BEFORE this command \
      ran. A digest you carry from an earlier round is a claim about what you \
      already saw, and only --expected-state can check it — which is what catches \
      a resumed shell reusing genesis state. Use --auto-state for the genesis \
      observation, where the tip is read fresh anyway, and keep --expected-state \
      for a closing observation made against a tip an earlier append returned.\n  \
      With both flags the tip is read twice, once for the sweep and once for the \
      append. If it moved in between, the append fails with the distinct \
      `review_state_tip_moved`, naming both digests — never a silent re-read.")]
pub struct PrReviewLoopObserveArgs {
    pub id: u64,
    /// Exact provider head SHA whose review observation is being appended.
    #[arg(long = "expected-head", value_name = "SHA", value_parser = clap::builder::NonEmptyStringValueParser::new())]
    pub expected_head: String,
    /// Merge envelope, observation array, correction, or stop supersession envelope.
    /// Rows require lifecycle identity; see the help footer for exact schemas.
    #[arg(long = "findings-file", value_name = "PATH")]
    pub findings_file: String,
    /// Exact current provider-visible state-chain tip; omit only for genesis.
    #[arg(long = "expected-state", value_name = "DIGEST")]
    pub expected_state: Option<String>,
    /// Read the current chain tip and compare-and-swap against it, instead of
    /// naming it with `--expected-state`. See this command's help footer for
    /// what this does NOT detect.
    #[arg(long = "auto-state", action = ArgAction::SetTrue, conflicts_with = "expected_state")]
    pub auto_state: bool,
    /// Run the full `--dry-run` verdict sweep before this live append and stop
    /// with `review_preflight_failed` if any rule fails.
    #[arg(long = "preflight", action = ArgAction::SetTrue)]
    pub preflight: bool,
    /// Human-readable delivery outcome to post in the same comment as this
    /// ledger record. Mutually exclusive with `--body-file`.
    #[arg(long, conflicts_with = "body_file")]
    pub body: Option<String>,
    /// Read the delivery outcome body from PATH (`-` reads stdin).
    #[arg(long = "body-file", value_name = "PATH")]
    pub body_file: Option<String>,
}
