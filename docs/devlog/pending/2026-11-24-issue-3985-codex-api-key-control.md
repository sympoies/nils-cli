## 2026-11-24 - Keep Codex control ready under API-key authentication
<!-- devlog-id: 2026-11-24-issue-3985-codex-api-key-control -->

### Result

- Managed Codex control admits keyed prompts when the ChatGPT-only rate-limit
  read is unavailable.

### Why / context

- Codex returns a specific JSON-RPC refusal for API-key accounts, so the
  initial read skips the usage wake and lets control become ready.

### Evidence

- The API-key fixture failed on main before readiness. After the fix, all 209
  focused app-server and prompt-outcome tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3985>
