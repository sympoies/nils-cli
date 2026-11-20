## 2026-11-18 - Recover verified idle state after Codex resume
<!-- devlog-id: 2026-11-18-codex-resume-idle-state -->

### Result

- Resume startup checks the bound provider thread on the subscribed control
  connection and records later turn starts.

### Why / context

- A resumed session could remain unverified and block idle-guarded operations.

### Evidence

- Synthetic resumed-thread regression and app-server tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3808>
