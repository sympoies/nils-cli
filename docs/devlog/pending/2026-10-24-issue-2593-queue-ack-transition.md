## 2026-10-24 - Require new queue evidence for Claude hand-off acknowledgements
<!-- devlog-id: 2026-10-24-issue-2593-queue-ack-transition -->

### Result

- Claude pane acknowledgements compare full whitespace-free prompt sighting
  counts before and after submission; unchanged sibling and identical queue
  entries remain unknown.

### Why / context

- A 20-character composer probe was also used as queue delivery proof, so an
  older sibling could hide a lost paste. Increasing the prefix cannot
  distinguish repeated hand-offs; the acknowledgement needs evidence created
  after the attempt.

### Evidence

- The synthetic endpoint regression first returned submitted for an absent
  same-prefix hand-off, then stayed unknown after the fix. All 50
  prompt-receipt tests and five structured-prompt tests pass, including
  provider-turn, transcript-queue and replay behavior.

### Links

- <https://github.com/sympoies/nils-cli/issues/2593>
