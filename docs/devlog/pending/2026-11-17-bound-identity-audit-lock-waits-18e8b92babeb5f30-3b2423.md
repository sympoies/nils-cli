## 2026-11-17 - Bound identity audit lock waits
<!-- devlog-id: 2026-11-17-bound-identity-audit-lock-waits-18e8b92babeb5f30-3b2423 -->

### Result

- Audit lock contention has a one-second limit with typed failure and
  optional-audit warning outcomes.

### Why / context

- A stopped lock holder must not indefinitely stall identity-gated operations,
  while required audit remains fail-closed.

### Evidence

- A disposable lock holder reproduced the unbounded wait before the fix;
  focused identity tests exercise bounded contention.

### Links

- <https://github.com/sympoies/nils-cli/issues/3635>
