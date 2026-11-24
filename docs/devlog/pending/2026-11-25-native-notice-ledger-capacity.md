## 2026-11-25 - Release live notice capacity held by archived unknown outcomes
<!-- devlog-id: 2026-11-25-native-notice-ledger-capacity -->

### Result

- At capacity, the native notice store archives terminal unknown outcomes while
  keeping their keys and operation IDs consumed.

### Why / context

- A completed but uncertain submission needs durable replay protection without
  permanently occupying a live claim slot.

### Evidence

- The pre-fix native fixture with prior turns refused with claim-ledger-full
  and the mixed compaction fixture retained an extra terminal unknown.

### Links

- <https://github.com/sympoies/nils-cli/issues/3911>
