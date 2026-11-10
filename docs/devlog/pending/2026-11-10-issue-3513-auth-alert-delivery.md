## 2026-11-10 - Retain deliverable authentication alerts and consumer receipts
<!-- devlog-id: 2026-11-10-issue-3513-auth-alert-delivery -->

### Result

- Owner-unavailable incidents now retain an independent consumer record with
  configured person/watchdog obligations and explicit delivery
  acknowledgements.

### Why / context

- An observation-spool entry cannot establish that a person was notified. The
  external consumer needs durable pending work and immutable per-alert target
  requirements without relying on the quarantined runtime.

### Evidence

- Retained baseline RED: an owner-unavailable daemon tick writes a spool entry
  but no deliverable authentication alert record.

### Links

- <https://github.com/sympoies/nils-cli/issues/3513>
