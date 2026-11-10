## 2026-11-10 - Guard Claude mailbox wakes without a native receiver
<!-- devlog-id: 2026-11-10-claude-mailbox-text-wake -->

### Result

- Use a single-line guarded text wake when Claude native notices are
  unavailable for a known launch or configuration reason, retaining the
  specific cause in the notification receipt.

### Why / context

- Native authorization timeouts remain retryable under the existing parking
  contract. Configuration absence needs a separate pre-attempt route; a
  nonempty composer, including an old draft, must remain untouched, and an
  unknown attempted generation must remain consumed.

### Evidence

- Before production edits, both configuration regressions failed against the
  main baseline. The four configuration fallback tests then passed, covering
  draft preservation, load-time edits, one submission and unknown-outcome
  containment.

### Links

- <https://github.com/sympoies/nils-cli/issues/3523>
