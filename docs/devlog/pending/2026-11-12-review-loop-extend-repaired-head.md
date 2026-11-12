## 2026-11-12 - review-loop extend binds a repaired head
<!-- devlog-id: 2026-11-12-review-loop-extend-repaired-head -->

### Result

- forge-cli pr review-loop extend now consumes an approved extension when the
  provider head is a repair that descends from the stopped head; the record
  binds the stopped head and unrelated heads stay refused
- extend --dry-run now reads the pull request and ledger and reports the same
  head and tip conflicts as the real call

### Why / context

- After a repair push the ledger head and provider head differ, so extend
  failed with review_state_conflict on either --expected-head value

### Evidence

- focused forge-cli extend unit tests; test-first record retained

### Links

- sympoies/nils-cli#3506
