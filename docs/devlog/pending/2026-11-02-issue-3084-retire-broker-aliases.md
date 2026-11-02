## 2026-11-02 - Retire the broker response field aliases
<!-- devlog-id: 2026-11-02-issue-3084-retire-broker-aliases -->

### Result

- agent-session reads account broker responses with the current account and
  plan field names only; the serde aliases for the older nickname and
  chatgpt_plan_type names, their doc comment and their two allowlist rows are
  removed.

### Why / context

- The searched broker source history never emitted the older names as response
  keys, so a copy built from that source cannot either (not a claim about every
  installed producer); the usage fixture that still used them now uses the
  current names, and a test asserts the older names are not read.

### Evidence

- The rejection test failed before the change and passes after; codex_account
  unit tests, the serve usage integration tests, fmt, clippy for agent-session,
  the own-compat audit and the file-size audit pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3084>
