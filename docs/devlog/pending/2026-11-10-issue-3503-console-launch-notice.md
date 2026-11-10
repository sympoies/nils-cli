## 2026-11-10 - Carry Console launch notices to the target receipt writer
<!-- devlog-id: 2026-11-10-issue-3503-console-launch-notice -->

### Result

- Console starts carry optional notice metadata to the target and fresh starts
  report written, not-requested, or unavailable without exposing the receipt.

### Why / context

- The origin encoded the dispatch prompt but dropped the explicit notice
  fields, so target startup skipped the receipt. Plain omitted fields remain
  the current ordinary-start contract; Console edge forwarding is coordinated
  separately.

### Evidence

- Retained baseline target-create RED (exit 101); target/direct notice-start
  integration suite passed all four tests after the fix; sender metadata
  transport test passed; scoped Clippy passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3503>
- <https://github.com/sympoies/salon/issues/406>
