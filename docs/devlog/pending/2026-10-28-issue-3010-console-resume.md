## 2026-10-28 - Authenticated Console provider resume transport
<!-- devlog-id: 2026-10-28-issue-3010-console-resume -->

### Result

- Add read-only Console resume preflight and a versioned target create endpoint
  that retains account-bound admission and operation replay protection.

### Why / context

- Resume checks must not produce operations or silently fall back to fresh
  conversations through older create parsers.
- CLI replay reaches the retained operation receipt before provider admission;
  a distinct origin resume route keeps older daemons from interpreting it as a
  fresh start.

### Evidence

- Retain meaningful RED/GREEN evidence for HTTP contracts, replay, compatibility
  and terminal transport details. Scoped local validation and hosted CI gate
  delivery.
- The signed repair head passed its complete scoped package gate: 2,579 tests
  passed, one skipped. Current-main integration preserves that receipt and
  adds bounded transport coverage for optional history scope and pending account
  transitions.
- Compose with account-switch recovery and session metadata refresh on current
  main. Preserve explicit bound-account intent settlement under source guards;
  preflight remains observational.
- Real-provider recall and deployed Console role acceptance remain a separate
  gate after the matching Console relay is available.

### Links

- <https://github.com/sympoies/nils-cli/issues/3010>
