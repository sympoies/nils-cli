## 2026-11-06 - Reuse upload ownership proofs within history reads
<!-- devlog-id: 2026-11-06-issue-3372-upload-ownership -->

### Result

- History projections reuse the existing archive and live-session ownership
  proof per managed session within each read; upload descriptors and
  file-opening checks are preserved.

### Why / context

- Repeated quoted paths previously reloaded the same ownership records. The
  cache ends with the read so a later read observes ownership changes.

### Evidence

- Focused HTTP regression and refusal tests added; validation results recorded
  in the linked pull request.

### Links

- <https://github.com/sympoies/nils-cli/issues/3372>
