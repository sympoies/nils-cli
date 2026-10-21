## 2026-10-21 - Report skipped required checks apart from passes in forge-cli pr checks
<!-- devlog-id: 2026-10-21-issue-2643-skipped-required-checks -->

### Result

- forge-cli pr checks and pr wait-checks expose required_skipped,
  required_neutral and a skipped_required list, with a text marker, so a
  skipped or neutral required check is no longer indistinguishable from a real
  pass.

### Why / context

- Skipped and neutral required checks satisfy GitHub rulesets, and deferred
  full runs now conclude skipped, so gate_state=success alone does not show
  that the work ran.

### Evidence

- A fixture with skipped and neutral required checks failed before the change
  and passes after; gate_state, success_count and the existing fields are
  unchanged. Focused forge-cli check tests and clippy passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2643>
