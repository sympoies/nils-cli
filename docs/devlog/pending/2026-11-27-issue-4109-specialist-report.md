## 2026-11-27 - Identify invalid specialist reports in designated review admission
<!-- devlog-id: 2026-11-27-issue-4109-specialist-report -->

### Result

- Review handoff check reports the specialist validation cause, and forge-cli
  validates reports declared by the specialist marker before publication.

### Why / context

- A malformed approval could reach the provider and later produce a generic
  missing-review refusal even though a current-head review was present.

### Evidence

- Fake-provider tests reproduced the missing Evidence reviewed diagnostic and
  pre-publication validation gaps on the baseline, then passed with the shared
  validator.

### Links

- <https://github.com/sympoies/nils-cli/issues/4109>
