## 2026-10-25 - Explain recipient authority refusals without consuming mail
<!-- devlog-id: 2026-10-25-issue-2928-mailbox-authority-refusal -->

### Result

- Local mailbox and broker identity refusals retain coordination-unauthorized
  and add fixed reason codes for supplied credentials and authenticated
  broker/runtime fences, plus non-destructive preservation guidance.

### Why / context

- A generic refusal previously hid the failed check after an earlier successful
  inbox read. Synthetic transitions reproduce the diagnostic gap; the
  historical trigger and original client pin remain unknown.

### Evidence

- Two pre-fix CLI fixtures failed on missing reason codes; three
  authority-transition fixtures and all 57 existing coordination integration
  tests pass after repair. Refusals retain unread payloads and redact
  capability, incarnation, path and body canaries.

### Links

- <https://github.com/sympoies/nils-cli/issues/2928>
