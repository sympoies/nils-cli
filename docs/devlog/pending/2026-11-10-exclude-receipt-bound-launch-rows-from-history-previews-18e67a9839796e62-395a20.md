## 2026-11-10 - Exclude receipt-bound launch rows from History previews
<!-- devlog-id: 2026-11-10-exclude-receipt-bound-launch-rows-from-history-previews-18e67a9839796e62-395a20 -->

### Result

- History prompt previews skip the launch row when its physical receipt binding
  matches.

### Why / context

- The same pasted content remains a user prompt without the launch receipt.

### Evidence

- The focused launch integration regression reproduced the preview failure
  before the fix and passed afterward.

### Links

- <https://github.com/sympoies/nils-cli/issues/3488>
