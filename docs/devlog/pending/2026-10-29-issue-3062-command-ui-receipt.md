## 2026-10-29 - Separate observed owner command UI from prompt submission
<!-- devlog-id: 2026-10-29-issue-3062-command-ui-receipt -->

### Result

- Observed owner command UI transitions have a terminal command_dispatched
  receipt with submitted false; generic unknown dialogs retain their prior
  result.
- Only authenticated Claude login is quarantine recovery. Managed Codex login
  keeps quarantine and account-authority refusal, with account-switch guidance.

### Why / context

- The native Status baseline processed the command while generic composer
  confirmation remained unconfirmed. A dispatched UI receipt must not claim
  model submission, settings changes or completed login.

### Evidence

- Repair RED recorded two observable failures and an explicit Codex
  quarantine-precedence failure. Final focused validation passes: 59 receipt
  tests, 12 native-send tests, four quarantine tests, package clippy,
  formatting and strict file-size audit.

### Links

- <https://github.com/sympoies/nils-cli/issues/3062>
- <https://github.com/sympoies/nils-cli/pull/3064>

### Follow-ups

- The native fixture contains original structural rows only, with body gaps
  unreconstructed and production capture-space parity unknown. Native candidate
  acceptance and consumer adaptation remain separately owned.
