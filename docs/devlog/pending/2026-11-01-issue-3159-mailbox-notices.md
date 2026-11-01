## 2026-11-01 - Mailbox reminders use the canonical notice path
<!-- devlog-id: 2026-11-01-issue-3159-mailbox-notices -->

### Result

- Native mailbox reminders use mailbox.wake notices under the existing writer
  gate, retaining coalesced unread counts and shared controller/hook generation
  claims.

### Why / context

- Unmarked native reminders were filtered from visible notice history. Explicit
  unsupported fallback is retained; attempted or uncertain notices cannot
  produce a second plain prompt.

### Evidence

- Meaningful pre-edit failures at idle, working and authenticated hook
  boundaries; released CLI isolated transport proof. Focused source validation
  and separately scheduled provider/UI acceptance are distinct.

### Links

- <https://github.com/sympoies/nils-cli/issues/3159>
