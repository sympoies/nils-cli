## 2026-11-25 - Expire abandoned native notice claims
<!-- devlog-id: 2026-11-25-expire-native-notice-claims -->

### Result

- Seal expired and retired native notice claims as unknown and release their
  live slots.

### Why / context

- An unfinished claim could otherwise consume one of the finite slots
  permanently.

### Evidence

- The saturated ledger regression was red before the repair. Lifecycle tests
  cover the deadline, stopped and replaced launches, preserved positive proof,
  and consumed keys.

### Links

- <https://github.com/sympoies/nils-cli/issues/4030>
