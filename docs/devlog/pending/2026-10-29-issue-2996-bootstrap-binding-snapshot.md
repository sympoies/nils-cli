## 2026-10-29 - Fence bootstrap account reads to the current request binding
<!-- devlog-id: 2026-10-29-issue-2996-bootstrap-binding-snapshot -->

### Result

- Bootstrap account reads capture the live binding for their exact runtime once
  per request and retain account/revision refusal fences throughout the wait.

### Why / context

- The proxy launch record can precede a completed same-account credential
  refresh. Comparing each later request with that snapshot incorrectly reports
  an initial-binding replacement even when the current account is bound.

### Evidence

- The focused regression first reproduced account_authority_unavailable /
  initial-binding-replaced at bound revision 3 after a successful account read
  and same-account refresh. The focused bootstrap and typed-cause suite passes
  with runtime, account, and revision replacement refusals preserved.

### Links

- <https://github.com/sympoies/nils-cli/issues/2996>
