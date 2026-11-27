## 2026-11-28 - Diagnose test-change detector version mismatches
<!-- devlog-id: 2026-11-28-test-change-detector-version -->

### Result

- Review handoff confirmations record the detector version and identify version
  mismatches.

### Why / context

- Mixed detector versions can produce different confirmation digests for the
  same comparison.

### Evidence

- Regression coverage exercises a confirmation from a different detector
  version.

### Links

- <https://github.com/sympoies/nils-cli/issues/4171>
