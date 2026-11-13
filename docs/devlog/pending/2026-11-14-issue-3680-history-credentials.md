## 2026-11-14 - Mask credentials consistently in history display forms
<!-- devlog-id: 2026-11-14-issue-3680-history-credentials -->

### Result

- Both masked and owner history forms redact URL query credentials and shared
  assignment/token patterns while retaining owner path and URL context.

### Why / context

- The existing path detector could miss URLs containing query equals signs, and
  masked summaries did not apply the shared token masker. Reusing owner masking
  requires preserving already-redacted assignments and Authorization schemes
  during repeated projection.

### Evidence

- Red: 18 focused tests passed and the URL-query and bare-token regressions
  failed (exit 101). Green: all 20 focused history-parts tests passed,
  including unchanged provider snapshot bytes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3680>
