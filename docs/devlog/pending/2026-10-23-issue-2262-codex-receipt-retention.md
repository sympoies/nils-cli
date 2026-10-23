## 2026-10-23 - Codex refresh receipt retention for closed connections
<!-- devlog-id: 2026-10-23-issue-2262-codex-receipt-retention -->

### Result

- Closed-connection terminal refresh receipts expire after the retry window; a
  stable runtime can refresh beyond sixty-four completed connections.

### Why / context

- An unpruned receipt store permanently exhausted refresh capacity. Proven
  connection retirement permits bounded cleanup while live replay identities
  and uncertain effects remain protected.

### Evidence

- The real-control-loop regression failed on refresh 65 before implementation;
  focused post-fix validation is recorded in the linked PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/2262>
