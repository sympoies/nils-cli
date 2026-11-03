## 2026-11-03 - Clarify broker refusal diagnostics
<!-- devlog-id: 2026-11-03-clarify-broker-refusal-diagnostics-18e46fb60abd5b07-29fca2 -->

### Result

- Broker refusal hints identify the relevant status or capability check, and
  missing launch bindings have a distinct message.

### Why / context

- Launch binding identity cannot establish heartbeat freshness or validate the
  selected runtime capability.

### Evidence

- A stale-heartbeat regression failed before the change; capability unit and
  launch-binding integration tests passed after it.

### Links

- <https://github.com/sympoies/nils-cli/issues/3265>
