## 2026-11-10 - Document synthetic authentication quarantine acceptance
<!-- devlog-id: 2026-11-10-issue-3005-auth-quarantine-acceptance -->

### Result

- Expose the existing Claude recheck/schedule and Codex queued-repair fixtures
  as one resource-gated nextest command.

### Why / context

- The reported production paths already exist, but acceptance remained unrun
  because testers lacked a documented disposable fixture. Separate synthetic
  admission proof from native provider acceptance.

### Evidence

- Source reconciliation on main 8588885b confirms merged PRs 3475 and 3515
  cover the reported cases; the recipe names repaired and unrepaired assertions
  and uses synthetic broker credentials only.

### Links

- <https://github.com/sympoies/nils-cli/issues/3005>
