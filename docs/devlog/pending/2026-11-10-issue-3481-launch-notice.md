## 2026-11-10 - Bind Claude launch notices to the first history row
<!-- devlog-id: 2026-11-10-issue-3481-launch-notice -->

### Result

- Fresh Claude launch notices retain a private launch receipt for first-row
  history projection.

### Why / context

- Provider paste envelopes move the notice header away from the start of the
  row; matching the launch receipt preserves the producer boundary.

### Evidence

- Synthetic launch regression and receipt mismatch controls.

### Links

- <https://github.com/sympoies/nils-cli/issues/3481>
