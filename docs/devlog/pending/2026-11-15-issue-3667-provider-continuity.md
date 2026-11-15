## 2026-11-15 - Publish owned provider lifecycle continuity
<!-- devlog-id: 2026-11-15-issue-3667-provider-continuity -->

### Result

- Running session snapshots expose up to sixteen persisted clear, restart and
  resume links for the same managed session.

### Why / context

- Clear changes the provider conversation before restart changes the
  incarnation; the relay needs evidence of both transitions to advance an
  existing ownership grant without rebinding. Observations and rebinds never
  create proof.

### Evidence

- A two-root synthetic fixture completed clear and restart, then failed the
  missing-continuity assertion before implementation; the same snapshot test
  passes with the daemon change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3667>
