## 2026-10-28 - Recover deleted Claude sources through read-only account authority
<!-- devlog-id: 2026-10-28-issue-3012-deleted-source-resume -->

### Result

- Direct Claude resume can create a new managed record from deleted-source
  history when an explicit broker account resolves its exact existing history
  root.

### Why / context

- A removed managed binding cannot authorize a guessed account directory. A
  read-only broker verb supplies account authority; final admission runs again
  after preparation so newly appearing writers cannot bypass it.

### Evidence

- Three new CLI regressions failed before production edits; all eleven
  start-resume CLI cases now pass, including duplicate files and
  preparation-time history/account/source races. Real-provider recall remains a
  separate scheduled gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3012>
