## 2026-11-09 - Complete session preview refresh after list cancellation
<!-- devlog-id: 2026-11-09-issue-3451-cancelled-preview -->

### Result

- Cached session preview work completes independently of the list request, so
  cancellation cannot strand the latest real Claude prompt in a pending state.

### Why / context

- The large-transcript and equivalent-account-path shape alone recovered
  correctly. Cancellation after the single-flight flag was set dropped the
  state transition while blocking work continued.

### Evidence

- The synthetic large-transcript cancellation regression failed before
  production edits with a permanently pending preview; retained test-first
  evidence records the failing command and assertion.

### Links

- <https://github.com/sympoies/nils-cli/issues/3451>
