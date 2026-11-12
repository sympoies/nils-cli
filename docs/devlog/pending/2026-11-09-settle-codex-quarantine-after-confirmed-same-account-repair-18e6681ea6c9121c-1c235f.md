## 2026-11-09 - Settle Codex quarantine after confirmed same-account repair
<!-- devlog-id: 2026-11-09-settle-codex-quarantine-after-confirmed-same-account-repair-18e6681ea6c9121c-1c235f -->

### Result

- A confirmed daemon account repair settles the matching Codex authentication
  incident.

### Why / context

- The queued repair can apply without a prompt, but its prior incident
  otherwise continues to fence input; newer failures remain fenced.

### Evidence

- The disposable reconciliation regression reproduces the retained quarantine
  before the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3005>
