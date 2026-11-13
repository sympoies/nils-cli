## 2026-11-13 - Owner display forms retain masked history output
<!-- devlog-id: 2026-11-13-issue-3677-owner-history-parts -->

### Result

- History tool and question parts carry optional owner display text with
  independent bounds and the same aggregate byte budget.

### Why / context

- The consumer selects owner display text while fixed pre-change snapshots
  preserve the masked wire values; credential keys and owner URL credentials
  remain hidden.

### Evidence

- Designated review exposed credential assignment and scheme-less userinfo disclosure; synthetic
  canaries reproduce the assignment failure before repair, and all 19 focused history-parts tests
  pass after repair.

- Pre-implementation red: 25 passed and 3 new owner tests failed. Focused
  history-parts tests: 19 passed, including HTTP serialization and exact masked
  fixture baselines.

### Links

- <https://github.com/sympoies/nils-cli/pull/3678>
