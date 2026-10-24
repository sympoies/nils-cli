## 2026-10-24 - Preserve original image download filenames
<!-- devlog-id: 2026-10-24-preserve-original-image-download-filenames-18e16051be35b704-334d47 -->

### Result

- Authenticated history image responses include sanitized UTF-8
  Content-Disposition filenames when known; embedded images and legacy uploads
  with unknown names omit the header.

### Why / context

- The upload storage name intentionally loses display-name characters.
  Session-owned bounded filename metadata retains the original leaf separately,
  and reads remove path components and controls again before header encoding.

### Evidence

- Real uploaded-name and unsafe-name HTTP regressions failed first because the
  header was absent; all three filename regressions now pass with unchanged
  image bytes, MIME and length.

### Links

- <https://github.com/sympoies/nils-cli/issues/2892>
