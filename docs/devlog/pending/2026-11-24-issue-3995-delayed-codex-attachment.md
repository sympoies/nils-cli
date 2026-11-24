## 2026-11-24 - Keep Codex control connected during a delayed fresh attachment
<!-- devlog-id: 2026-11-24-issue-3995-delayed-codex-attachment -->

### Result

- A fresh control waits for the TUI proxy to attest the thread before starting
  loaded-thread verification.

### Why / context

- A slow app-server startup could exhaust the discovery budget before the first
  thread existed, causing keyed structured prompts to be refused.

### Evidence

- The delayed-offline peer regression failed on main and passed with the fix.
  The already-attached API-key and ChatGPT cases passed in the same focused
  suite.

### Links

- <https://github.com/sympoies/nils-cli/issues/3995>
