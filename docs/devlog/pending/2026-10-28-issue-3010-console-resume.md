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
- Preserve original account-selection consent separately from the resolved
  execution account. Two target/transport regression contracts pass after
  reproducing an unintended launch when a switch was queued after preflight;
  omitted-account refusal preserves source state, while an explicit selection
  still launches once and replays its receipt without another upstream request.
- Read daemon fields through `State.0` before closure capture to preserve the
  static-analysis source boundary without changing HTTP admission behavior.
- Compose Console transport with broker-authorized deleted-source history and
  account/shared scope. Preserve original consent through post-preparation
  admission, pin the resolved cwd, and settle only the final guarded source
  snapshots. Regression coverage includes source disappearance after preflight
  and a pending switch introduced by credential preparation.
- Real-provider recall and deployed Console role acceptance remain a separate
  gate after the matching Console relay is available.

### Links

- <https://github.com/sympoies/nils-cli/issues/3010>
