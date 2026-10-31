## 2026-10-31 - Add admitted-service Console starts with owner receipts
<!-- devlog-id: 2026-10-31-admitted-service-console-starts -->

### Result

- An explicitly admitted service can start a Console-owned root session for an
  admitted owner and target with a durable owner receipt, without a model turn
  or another session's credential.

### Why / context

- Managed-caller and worker-delegation starts could not serve a deterministic
  integration that has no session, and mailbox service credentials only submit
  messages.

### Evidence

- Meaningful RED was the refused-today path: a service start failed to parse
  and the unmanaged reconcile probe returned console-start-unmanaged. Loopback
  daemon acceptance against a fake aggregator now covers success, replay,
  conflict, refusals, uncertain-result reconciliation and credential
  separation; managed-caller, worker-delegation and mailbox-service regressions
  pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3109>

### Follow-ups

- The Console aggregator must accept the service-selector create and board-scan
  shapes in the specification before end-to-end use.
