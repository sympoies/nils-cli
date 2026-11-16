## 2026-11-16 - Auth recovery waits for elapsed publication deadline
<!-- devlog-id: 2026-11-16-issue-3747-credential-wait -->

### Result

- Claude recovery allows newer local publication for thirty elapsed seconds
  without an independent poll-count cutoff; the existing same-account lifecycle
  restarts once and never replays input.

### Why / context

- Fast controller observations could consume eight polls before delayed
  replication completed. Equal, older, other-account and incomplete publication
  metadata remain ineligible.

### Evidence

- Retained RED: two regression assertions failed before production edits.
  Focused Rust 1.100.0 recovery suite: 47 passed using synthetic fixtures and
  injected tick time.

### Links

- <https://github.com/sympoies/nils-cli/issues/3747>
