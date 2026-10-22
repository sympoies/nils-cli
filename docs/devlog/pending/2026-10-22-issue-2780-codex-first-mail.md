## 2026-10-22 - Probe idle Codex sessions before their first turn on mail
<!-- devlog-id: 2026-10-22-issue-2780-codex-first-mail -->

### Result

- Never-turned Codex sessions are candidates for mailbox reminders through the
  managed app-server route.

### Why / context

- Starting activity has no completed turn; native bound-conversation idle
  verification permits reminders without treating startup as proof of idleness.

### Evidence

- Captured a failing focused regression for starting-session candidate
  selection; focused activity and native idle probe validation accompanies the
  PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/2780>
