## 2026-11-01 - Project queued Claude upload references in history
<!-- devlog-id: 2026-11-01-issue-3169-queued-upload-history -->

### Result

- Human Claude queued-string prompts project same-session quoted raster uploads
  with ordered captions, user origin and opaque upload references; selector
  reads use the same carrier semantics.

### Why / context

- Claude Code 2.1.316 can store a quoted upload in attachment.prompt instead of
  message.content. Reusing the existing human queued selector and ordered parts
  walker avoids a second classifier and ignores repeated rendered text without
  rewriting transcript or native input.

### Evidence

- Before production edits, real loopback HTTP returned zero queued image
  descriptors and the frozen route suite had three meaningful assertion
  failures. After the change, 49 focused tests passed and source-built
  idle/queued HTTP upload-history-image reads returned the exact synthetic JPEG
  bytes with authentication and unchanged transcripts.

### Links

- <https://github.com/sympoies/nils-cli/issues/3169>

### Follow-ups

- The structural carrier evidence is attributed; native/client acceptance, full
  merge gates and containing release remain separately owned under issue #3169.
