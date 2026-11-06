## 2026-11-06 - Preserve service Console root identity
<!-- devlog-id: 2026-11-06-preserve-service-console-root-identity-18e53e112cdfecab-1c65cb -->

### Result

- Admitted service starts carry validated role and work identity into root
  creation and owner receipts.

### Why / context

- Deterministic service restarts need the same identity validation as direct
  starts.

### Evidence

- The synthetic service Console start integration suite exercises creation,
  reconciliation and refusal.

### Links

- <https://github.com/sympoies/nils-cli/issues/3375>
