## 2026-10-29 - Alert parentless authentication incidents through the observation spool
<!-- devlog-id: 2026-10-29-issue-3061-parentless-auth-alert -->

### Result

- A session with no effective parent now records one deduplicated
  observation-spool event per authentication incident and final result, with a
  typed owner-recovery action; recording is never reported as delivery.
- Authentication incidents record a fixed settlement path: model health,
  explicit credential recheck, or controller.

### Why / context

- A parentless coordinator could not receive its own incident through the
  mailbox, and settlement receipts did not say which path settled them.

### Evidence

- Focused regressions cover parentless, unavailable then restored alert route,
  restored owner route, ordinary child owner, quarantine guard, settlement
  paths and secret-free output.

### Links

- <https://github.com/sympoies/nils-cli/issues/3061>
