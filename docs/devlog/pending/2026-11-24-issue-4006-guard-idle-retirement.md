## 2026-11-24 - Guard idle agent-session retirement
<!-- devlog-id: 2026-11-24-issue-4006-guard-idle-retirement -->

### Result

- Add incarnation-fenced idle retirement for supported managed Codex sessions
  with an optional unread-mail predicate.

### Why / context

- A list snapshot cannot fence resume, native provider work, or mailbox
  delivery. The owner validates current state under the same lifecycle and
  provider locks used by those operations.

### Evidence

- Twenty-eight focused synthetic CLI, stopped-delete, provider compaction, and
  clear admission tests passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/4006>
