## 2026-10-24 - Handle vanished procfs identities during enumeration
<!-- devlog-id: 2026-10-24-issue-2873-process-group-exit-race -->

### Result

- Treat ESRCH while reading a Linux procfs process identity as process absence,
  alongside ENOENT. Other read and parsing failures remain unverified.

### Why / context

- A process can be reaped after its stat inode is opened but before it is read.
  Procfs then reports ESRCH, which the reader incorrectly treated as a failed
  verification. A disappearing unrelated process could invalidate an entire
  runtime membership scan and intermittently block stop/resume under load.

### Evidence

- Instrumented load failures recorded ESRCH before a runtime-unverified error.
  A deterministic descriptor regression opens stat while a child is alive,
  reaps it, then reads the retained descriptor; the previous reader returns
  VerificationFailed instead of absence. The regression passes after the fix,
  including the retained ENOENT behavior and refusal of permission errors.

### Links

- <https://github.com/sympoies/nils-cli/issues/2873>
