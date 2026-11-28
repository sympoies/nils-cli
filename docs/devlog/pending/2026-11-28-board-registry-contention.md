## 2026-11-28 - Keep board relay reads out of the registry writer queue
<!-- devlog-id: 2026-11-28-board-registry-contention -->

### Result

- Both board relay authentication hops verify the current capability and
  incarnation from the committed registry snapshot before considering the
  locked path.

### Why / context

- A healthy relay could time out twice on unrelated registry writers even
  though the network request was already outside the lock. Snapshot refusals
  still take the existing locked path, and mailbox mutations are unchanged.

### Evidence

- In an isolated loopback fixture, released agent-session 1.45.0 succeeded
  without contention but both authenticated paths returned
  coordination-lock-timeout while the unrelated fixture lock was held.

### Links

- <https://github.com/sympoies/nils-cli/issues/4195>
