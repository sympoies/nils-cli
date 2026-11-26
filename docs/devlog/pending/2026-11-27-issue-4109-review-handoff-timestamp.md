## 2026-11-27 - Admit marker-bound current-head approvals with tied timestamps
<!-- devlog-id: 2026-11-27-issue-4109-review-handoff-timestamp -->

### Result

- Designated review admission accepts an APPROVED current-head report with the
  active handoff digest when the provider timestamps tie.

### Why / context

- Whole-second provider timestamps can make a valid approval appear
  simultaneous with its assignment, including after same-head re-registration.

### Evidence

- A fake-provider regression reproduced the refusal before the fix and passed
  after it, with controls for the surrendered generation, an unknown digest,
  another head, and an older timestamp.

### Links

- <https://github.com/sympoies/nils-cli/issues/4109>
