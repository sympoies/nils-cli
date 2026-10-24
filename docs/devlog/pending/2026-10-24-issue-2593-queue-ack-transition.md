## 2026-10-24 - Require new queue evidence for Claude hand-off acknowledgements
<!-- devlog-id: 2026-10-24-issue-2593-queue-ack-transition -->

### Result

- Claude pane acknowledgements compare full whitespace-free prompt sighting
  counts before and after submission; unchanged sibling and identical queue
  entries remain unknown. Acknowledgement snapshots share the paste's
  session-record lock, including concurrent identical requests.

### Why / context

- A 20-character composer probe was also used as queue delivery proof, so an
  older sibling could hide a lost paste. Increasing the prefix cannot
  distinguish repeated hand-offs; the acknowledgement needs evidence created
  after the attempt. A concurrent-identical-request regression also showed
  that a snapshot taken before acquiring the delivery lock reused another
  request's new entry; each snapshot therefore shares its own paste's
  serialized boundary.

### Evidence

- The synthetic endpoint regression first returned submitted for an absent
  same-prefix hand-off, then stayed unknown after the fix. All 51
  prompt-receipt tests and five structured-prompt tests pass, including
  provider-turn, transcript-queue and replay behavior. The concurrent fixture
  first confirmed both requests from one new queue entry, then confirmed only
  its owner after the baseline moved inside serialized delivery.

### Links

- <https://github.com/sympoies/nils-cli/issues/2593>
