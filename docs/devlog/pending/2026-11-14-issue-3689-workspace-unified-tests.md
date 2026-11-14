## 2026-11-14 - Catch workspace feature differences before merge
<!-- devlog-id: 2026-11-14-issue-3689-workspace-unified-tests -->

### Result

- Required merge-group nextest checks resolve workspace features while testing
  the exact affected packages once. PR pushes retain scoped testing; lint,
  binary builds, doctests and dependency caches remain scoped.

### Why / context

- Package-scoped all-features tests missed serde_json preserve_order enabled by
  another workspace member, allowing a JSON key-order regression to reach
  release validation. The measured compilation cost exceeded the per-push
  budget, so unified testing belongs to the required queue gate.

### Evidence

- At 6881c6d7 the two history-parts tests pass scoped and fail with workspace
  resolution; both pass on the corrected tree. The CI consumer suite passes all
  46 tests, including PR scoped selection, queue unification and failure
  propagation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3689>
