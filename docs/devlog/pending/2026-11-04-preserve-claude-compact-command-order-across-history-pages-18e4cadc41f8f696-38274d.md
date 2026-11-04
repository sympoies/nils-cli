## 2026-11-04 - Preserve Claude compact command order across history pages
<!-- devlog-id: 2026-11-04-preserve-claude-compact-command-order-across-history-pages-18e4cadc41f8f696-38274d -->

### Result

- Claude compact pairs retain stable row identities while paging and card
  previews show the command before its compaction.
- Repeated echoes attached to one pair produce one command; separate
  compactions and unattached commands remain distinct invocations.

### Why / context

- The provider records the boundary before the retained command echo, so paging
  position must be independent of row identity.

### Evidence

- The retained test-first regression reproduces inverted order before the fix;
  focused and local-fast verification are tracked with the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3271>
