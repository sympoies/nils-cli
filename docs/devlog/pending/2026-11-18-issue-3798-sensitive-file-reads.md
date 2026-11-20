## 2026-11-18 - Admit the sensitive file read hook handler
<!-- devlog-id: 2026-11-18-issue-3798-sensitive-file-reads -->

### Result

- Added block-sensitive-file-reads to the agent-hook runtime-kit handler
  allowlist.

### Why / context

- The shared hook needs a released dispatcher to recognize its handler ID
  before registration.

### Evidence

- The synthetic Codex and Claude dispatcher test failed at allowlist admission
  before the change. The dispatcher contract and policy parity suites passed
  after the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3798>

### Follow-ups

- Release nils-cli before the kit registers the handler.
