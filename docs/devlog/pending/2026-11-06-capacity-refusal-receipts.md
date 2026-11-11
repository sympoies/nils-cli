## 2026-11-06 - Preserve capacity refusals in Console start receipts
<!-- devlog-id: 2026-11-06-capacity-refusal-receipts -->

### Result

- Reconciliation retains the validated capacity refusal across daemon restarts.

### Why / context

- A definite refusal previously retained its retry state but lost the typed
  capacity details needed by callers.

### Evidence

- The pre-fix Console operation regression observed a missing failure code.

### Links

- <https://github.com/sympoies/nils-cli/issues/3379>
