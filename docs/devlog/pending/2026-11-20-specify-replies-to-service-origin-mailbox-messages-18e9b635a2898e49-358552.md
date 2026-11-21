## 2026-11-20 - Specify replies to service-origin mailbox messages
<!-- devlog-id: 2026-11-20-specify-replies-to-service-origin-mailbox-messages-18e9b635a2898e49-358552 -->

### Result

- Documented the service recipient contract and its synthetic acceptance plan.

### Why / context

- Review the admission, retirement and polling rules before adding local
  delivery or extending the relay.

### Evidence

- git diff --check passed on the spec and runbook changes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3797>

### Follow-ups

- Implement the local contract and synthetic tests in PR 2.
