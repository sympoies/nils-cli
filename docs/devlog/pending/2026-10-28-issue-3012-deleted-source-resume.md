## 2026-10-28 - Recover deleted Claude sources through read-only account authority
<!-- devlog-id: 2026-10-28-issue-3012-deleted-source-resume -->

### Result

- Direct Claude resume can create a new managed record from deleted-source
  account-local or shared history through an explicit execution account and
  read-only broker authority. Preflight v1 adds optional history_scope.
- Account-switch recovery remains available through explicit bound-account
  selection after final source/history admission, preserving both recovery
  paths when integrated together.

### Why / context

- A removed binding cannot authorize guessed history. Final admission after
  broker preparation rechecks account, history and stopped source writers.
- Keep the original user selection through that recheck so an internally
  resolved account cannot silently authorize cancellation. Settle source
  intent only after all final guards succeed.

### Evidence

- Original deleted-source and scope contracts had meaningful red first.
  Two additional composition regressions failed before integration edits;
  combined start-resume cases now pass 14 tests and account-switch cases pass
  17 tests. Original head verification remains distinct from integration
  validation. Real-provider recall remains a separate scheduled gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3012>
- <https://github.com/sympoies/nils-cli/pull/3036>
