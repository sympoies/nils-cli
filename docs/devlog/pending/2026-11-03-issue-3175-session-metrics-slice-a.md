## 2026-11-03 - Add explicit bounded session growth metrics
<!-- devlog-id: 2026-11-03-issue-3175-session-metrics-slice-a -->

### Result

- Added the authenticated single-session metrics endpoint with retained-file
  bytes, physical lines, completed boundaries, independent managed/provider
  ages and explicit unrecorded resume values.

### Why / context

- Complete counts require content reads, so one bounded scan is isolated from
  ordinary polling. Provider identity owners and fixed unknown reasons prevent
  cross-conversation observations or partial totals from appearing complete.

### Evidence

- 45 focused and directly affected synthetic tests passed through the
  repository resource gate, including privacy canaries, mutation detection,
  shared scanning and unchanged query discovery budgets. Native acceptance
  remains separately scheduled.

### Links

- <https://github.com/sympoies/nils-cli/issues/3175>
