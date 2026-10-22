## 2026-10-22 - Confirm queued image prompt delivery from Claude transcripts
<!-- devlog-id: 2026-10-22-issue-2624-queued-prompt-receipt -->

### Result

- Structured prompt receipts accept newly observed queued attachments using the
  same list-form text reader as history.

### Why / context

- Busy turns can absorb queued input before a pane read or new turn hook proves
  delivery.

### Evidence

- Receipt regression reproduced an unknown outcome before the fix; focused
  acknowledgement tests passed after the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/2624>
