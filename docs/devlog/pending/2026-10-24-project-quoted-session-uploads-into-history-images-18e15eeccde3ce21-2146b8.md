## 2026-10-24 - Project quoted session uploads into history images
<!-- devlog-id: 2026-10-24-project-quoted-session-uploads-into-history-images-18e15eeccde3ce21-2146b8 -->

### Result

- Codex history pages render same-session quoted raster uploads as ordered
  image parts with caption text and authenticated reads.

### Why / context

- Only managed session attachments qualify; working-directory files stay text.
  Upload selectors retain original quoted positions so deletion cannot select a
  neighboring image. Existing structured and Markdown selectors remain valid.

### Evidence

- The HTTP upload and refresh regression failed first with no image
  descriptors, then all four quoted-upload route regressions passed, including
  ownership, symlink, non-image, mixed-source and stale-selector checks.

### Links

- <https://github.com/sympoies/nils-cli/issues/2891>
