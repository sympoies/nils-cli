## 2026-11-22 - Surface untrusted observation spools in diagnose
<!-- devlog-id: 2026-11-22-issue-3845-diagnose-untrusted-spool -->

### Result

- Diagnose reports an untrusted spool in text and JSON and marks health
  degraded.

### Why / context

- A symlinked ancestor can cause observation writes to be refused before the
  spool exists. The diagnostic applies the same root checks in read-only mode.

### Evidence

- The synthetic symlinked-ancestor integration test failed on the missing
  diagnostic and passes with the reporting change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3845>
