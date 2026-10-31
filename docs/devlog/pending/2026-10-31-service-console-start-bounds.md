## 2026-10-31 - Bound the service Console-start route and state its rejection mapping
<!-- devlog-id: 2026-10-31-service-console-start-bounds -->

### Result

- The service Console-start route takes its concurrency slot only after the
  request body is complete, and the start owns that slot until it finishes, so
  an unfinished request holds no slot and a client disconnect cannot free one
  while its start still runs.
- Reconciliation counts a board record as a root candidate only when it has a
  `lineage` object whose `parent` is null.
- The specification and runbook state the real aggregator rejection mapping: a
  typed code is forwarded and records `not_created` with `safe_to_retry: true`;
  only a missing or unreadable answer is `console-start-unavailable` with an
  `ambiguous` receipt.

### Why / context

- Review of the admitted-service Console start found the slot taken before the
  body was read and released on disconnect, a lineage-less record counted as a
  root, and a specification sentence that did not match the forwarded codes.

### Evidence

- Meaningful RED for the route bound and the lineage rule: eight stalled
  callers starved an admitted service, a ninth start was admitted after eight
  disconnected clients, and a lineage-less record was reported as a candidate.
  The rejection mapping needed only the specification; a test now pins the
  recorded state and code.

### Links

- <https://github.com/sympoies/nils-cli/issues/3145>
