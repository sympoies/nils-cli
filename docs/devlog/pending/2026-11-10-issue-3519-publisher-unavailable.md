## 2026-11-10 - Release handoffs with an unavailable review publisher
<!-- devlog-id: 2026-11-10-issue-3519-publisher-unavailable -->

### Result

- Coordinator-only publisher-unavailable recovery advances the trusted
  generation and restores non-assigned merge policy; reviewer recovery remains
  unavailable.

### Why / context

- A final-approval assignment without a working publisher otherwise has no
  supported release path, even with explicit review-convergence opt-out.
  Revoked review checks remain unavailable and no observation is fabricated.

### Evidence

- Retained pre-fix fixture run verified both existing merge refusals and
  rejected the new reason; post-fix lifecycle and identity/head/state fixtures
  pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3519>
