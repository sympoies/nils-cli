## 2026-11-02 - Freeze the mailbox read event contract (#3171)
<!-- devlog-id: 2026-11-02-issue-3171-mailbox-read-events-contract -->

### Result

- Added the mailbox read events v1 contract and a consumer fixture corpus: one
  stable notice-role event per message, read and ack update it in place,
  ordering by transcript byte anchor, no separate original-body route.

### Why / context

- Producer and consumer need an immutable shared payload before either side
  builds; recording is display-only and adds no agent turn.

### Evidence

- Fixture test: cargo test -p nils-agent-session --test
  mailbox_read_events_contract

### Links

- <https://github.com/sympoies/nils-cli/issues/3171>
