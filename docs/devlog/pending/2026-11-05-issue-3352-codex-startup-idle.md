## 2026-11-05 - Settle prompt-free Codex startup from primary idle evidence
<!-- devlog-id: 2026-11-05-issue-3352-codex-startup-idle -->

### Result

- Preserve the primary thread/start idle status through the managed proxy and
  settle startup before forwarding the response; expose
  codex_startup_idle_unverified after 30 seconds without idle evidence.

### Why / context

- A prompt-free serve create has no later turn hook to replace the initial
  starting state. Default-model Codex launches already return idle metadata,
  but proxy projection discarded it.

### Evidence

- Verified serve-create/proxy RED (Starting instead of Waiting) and
  unverified-startup RED (Starting instead of Unknown) before production edits;
  scoped validation is reported in the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3352>
