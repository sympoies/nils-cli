## 2026-11-03 - Optional mailbox reply deadlines
<!-- devlog-id: 2026-11-03-issue-3198-reply-deadlines -->

### Result

- Mailbox sends can request a bounded reply deadline; audit reports
  overdue-reply until an accepted correlated reply.

### Why / context

- Reply obligations are independent of read/ack state. Source-time deadlines
  survive federation, and a durable reply marker prevents compacted reply
  history from recreating an obligation.

### Evidence

- Focused deadline CLI and library tests, mailbox audit and separate-root HTTP
  federation acceptance, and affected coordination library tests passed through
  the repository resource gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/3198>
