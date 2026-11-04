## 2026-11-05 - Expose original upload names in history
<!-- devlog-id: 2026-11-05-history-original-upload-names -->

### Result

- History image descriptors and typed image parts expose sanitized original
  names for proven managed uploads.

### Why / context

- Consumers can label uploaded images consistently across history reloads.

### Evidence

- Managed metadata uses the existing bounded no-follow read and filename
  sanitizer.

### Links

- <https://github.com/sympoies/nils-cli/issues/3331>
