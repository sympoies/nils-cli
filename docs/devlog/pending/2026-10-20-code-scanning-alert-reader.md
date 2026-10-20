## 2026-10-20 - Read code-scanning alert evidence for an exact ref and commit
<!-- devlog-id: 2026-10-20-code-scanning-alert-reader -->

### Result

- Added a read-only alert view with exact instance binding and optional
  corresponding analysis SARIF.

### Why / context

- Default-branch alert metadata cannot establish the source-to-sink evidence
  for a pull request commit; analysis-level evidence remains explicitly scoped
  and requires correlation.

### Evidence

- Captured three missing-command fixture failures before implementation;
  affected forge crate tests and identity fixtures pass, with strict Clippy
  clean.

### Links

- <https://github.com/sympoies/nils-cli/issues/2668>
