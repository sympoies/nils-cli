## 2026-11-15 - Adopt queued Codex switches after terminal usage limits
<!-- devlog-id: 2026-11-15-issue-3728-queued-codex-switch -->

### Result

- Explicit queued Codex account switches can use fenced authoritative
  usage-exhaustion idle evidence; blocked prompt admission exposes retryable
  recovery and account views expose the pending reason.

### Why / context

- Manual queueing cancels auto-resume, while the former terminal-evidence path
  required an automatic failover intent. A failed live idle probe could
  therefore retain the queue indefinitely. Usage failure identity must remain
  in turn state independently of auto-resume policy.

### Evidence

- Three retained synthetic regressions failed before production edits; the
  first focused fixed run passed all four switch, admission, projection and
  input-fence cases.

### Links

- <https://github.com/sympoies/nils-cli/issues/3728>
