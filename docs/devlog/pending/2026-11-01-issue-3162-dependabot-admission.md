## 2026-11-01 - Dependabot admission uses exact-head canonical review gates
<!-- devlog-id: 2026-11-01-issue-3162-dependabot-admission -->

### Result

- The privileged Dependabot merge route now requires a verified automation
  head, exact-head final approval and canonical forge review/ledger/native/CI
  admission from trusted workflow source.

### Why / context

- Aggregate approval can survive a push and hide a current blocking COMMENTED
  verdict. Human integration heads now return to steward delivery; the artifact
  app identity comes from the existing token action.

### Evidence

- The actual merge-step offline regression failed before the repair; the
  repaired contract passes refusal and approved bot/app controls without a
  provider mutation.

### Links

- <https://github.com/sympoies/nils-cli/issues/3162>
