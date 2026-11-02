## 2026-11-02 - Record mailbox reads as conversation events (#3171)
<!-- devlog-id: 2026-11-02-issue-3171-mailbox-read-events-producer -->

### Result

- message show and ack now record one display-only notice event per message in
  a per-conversation ledger, anchored at the transcript byte length; the
  history messages route interleaves them in every direction.

### Why / context

- Operators and Console need to see when a session read or acknowledged mail,
  without any new agent turn or input path.

### Evidence

- Focused tests: cargo test -p nils-agent-session --lib -- read_events (4
  failed before the change, 7 pass after)

### Links

- <https://github.com/sympoies/nils-cli/issues/3171>
