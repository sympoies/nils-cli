## 2026-10-25 - Guard maintainer timeline-comment replacement
<!-- devlog-id: 2026-10-25-issue-2936-comment-redaction -->

### Result

- Added explicit repository, author and SHA-256 guarded redaction with
  body-free inspection and verified readback.

### Why / context

- Provider-supported moderation needs a distinct path from normal own-comment
  edits; replacing current text leaves edit history intact and unresolved
  writes must not replay blindly.

### Evidence

- Official GitHub moderation and REST documentation verified; nine offline
  redaction boundary tests and five existing edit-guard tests pass. No live
  comment was mutated.

### Links

- <https://github.com/sympoies/nils-cli/issues/2936>
