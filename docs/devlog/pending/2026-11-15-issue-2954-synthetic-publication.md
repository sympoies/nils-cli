## 2026-11-15 - Add disposable publication-to-recovery acceptance
<!-- devlog-id: 2026-11-15-issue-2954-synthetic-publication -->

### Result

- Add an offline staged publisher and full structured-failure recovery harness,
  plus a publication-before-launch prevention model and bounded live
  fixture-owner recipe.

### Why / context

- A synthetic publication can prove consumer fencing and lifecycle behavior
  without supplying the missing supported live provider fixture. Keep
  publication, resident adoption, submission and provider response as separate
  proofs.

### Evidence

- Acceptance criteria recorded before implementation. The focused recovery
  suite passed 46 tests; a workspace-selected run passed both new acceptance
  tests, including JSON comparisons. Live provider rows remain UNRUN until a
  supported disposable provider and publisher exist.

### Links

- <https://github.com/sympoies/nils-cli/issues/2954>
