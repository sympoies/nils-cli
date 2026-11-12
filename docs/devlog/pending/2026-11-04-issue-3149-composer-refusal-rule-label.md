## 2026-11-04 - Name the failed rule in a composer-unobservable refusal
<!-- devlog-id: 2026-11-04-issue-3149-composer-refusal-rule-label -->

### Result

- A direct reply refused with composer-unobservable now names the rule that
  failed inside the input row step: region-not-single-row, footer-dialog-hint,
  queued-hint-not-admitted, paste-placeholder or suggestion-literal-unstyled;
  row-unrecognized remains only for an unlocatable row. Classification and
  refusal behavior are unchanged.
- The word stays a fixed token in error.details.observation and the refused
  receipt, and never carries pane text.

### Why / context

- Refs 3149: the text-only footer shape with a background shell segment
  classifies as an empty composer, so the reported refusal depends on styling,
  status-line content or width. The label shows which rule fires on the next
  field occurrence.

### Evidence

- Direct prompt tests cover each label on both routes with a private marker
  that must not appear in the response.

### Follow-ups

- The refusal cause for 3149 still waits for a styled structural capture; the
  wrapped-suggestion hypothesis stays open.
