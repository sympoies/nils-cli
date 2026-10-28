## 2026-10-28 - Authenticated Console provider resume transport
<!-- devlog-id: 2026-10-28-issue-3010-console-resume -->

### Result

- Add read-only Console resume preflight and a versioned target create endpoint
  that retains account-bound admission and operation replay protection.

### Why / context

- Resume checks must not produce operations or silently fall back to fresh
  conversations through older create parsers.

### Evidence

- Retain meaningful RED/GREEN evidence for HTTP contracts, replay, compatibility
  and terminal transport details. Focused resume validation passed 12 CLI tests
  and 151 library tests; scoped local validation and hosted CI gate delivery.
- Real-provider recall and deployed Console role acceptance remain a separate
  gate after the matching Console relay is available.

### Links

- <https://github.com/sympoies/nils-cli/issues/3010>
