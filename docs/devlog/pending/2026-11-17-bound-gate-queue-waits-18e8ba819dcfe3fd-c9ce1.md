## 2026-11-17 - Bound gate queue waits
<!-- devlog-id: 2026-11-17-bound-gate-queue-waits-18e8ba819dcfe3fd-c9ce1 -->

### Result

- Gate admission now expires at the configured queue timeout and releases its
  ticket.

### Why / context

- The same setting bounds contained-runner queue waits, so both admission paths
  share a timeout contract.

### Evidence

- Resource admission regression exercises timeout and ticket cleanup.

### Links

- <https://github.com/sympoies/nils-cli/issues/3250>
