## 2026-11-10 - Schedule bounded same-account Claude credential rechecks
<!-- devlog-id: 2026-11-10-issue-3513-claude-auto-settle -->

### Result

- The daemon retains a three-attempt credential-recheck schedule, fixed
  refusals and exhaustion notifications without changing accounts or restarting
  the runtime.

### Why / context

- An authentication quarantine cannot admit the turn that would otherwise
  observe healthy authentication; credential repair needs an independent
  bounded recheck. The session-record lock also protects queued account intent.

### Evidence

- A retained baseline RED showed auth_failed and zero daemon broker calls
  despite usable credentials; the daemon settlement fixture passes after the
  change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3513>
