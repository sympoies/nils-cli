## 2026-10-22 - Confirm rewritten Claude image queues from observed composer evidence
<!-- devlog-id: 2026-10-22-issue-2624-image-queue-representation -->

### Result

- Bind busy image reply acknowledgement to the image label observed before
  Enter, then require new matching queue evidence. Retain a redacted Claude
  Code 2.1.308 recording and receipt replay regressions.

### Why / context

- A quoted image path is rewritten to an image label in the early enqueue. The
  list-form attachment can be appended after the seven-second receipt window
  while retaining the earlier provider timestamp; fixtures that kept text
  unchanged and appended immediately missed both differences.

### Evidence

- The recorded-shape receipt regression failed with HTTP 500 unknown before the
  fix and passed with HTTP 200 submitted after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/2624>
