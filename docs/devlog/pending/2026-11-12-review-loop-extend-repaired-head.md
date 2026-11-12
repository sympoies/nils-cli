## 2026-11-12 - review-loop extend binds a repaired head
<!-- devlog-id: 2026-11-12-review-loop-extend-repaired-head -->

### Result

- forge-cli pr review-loop extend now consumes an approved no-progress-ceiling
  extension when the provider head is a repair that descends from the ledger
  head; the record binds the ledger head and unrelated heads stay refused. A
  durable hard stop still accepts only the stopped head (supersession owns a
  repaired head there)
- extend --dry-run now reads the pull request and ledger and reports the same
  head and tip conflicts as the real call

### Why / context

- After a repair push the ledger head and provider head differ, so extend
  failed with review_state_conflict on either --expected-head value

### Evidence

- focused forge-cli extend unit tests; test-first record retained

### Links

- sympoies/nils-cli#3506
