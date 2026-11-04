## 2026-11-05 - Archive finished native notice claims without losing attestations
<!-- devlog-id: 2026-11-05-issue-3330-native-claim-lifecycle -->

### Result

- The native store caps live claims, seals eligible finished records into
  hash-linked immutable segments and keeps permanent compact key indexes.
- Retired segments are sealed `0400` after durable pointer publication; live data
  and metadata remain `0600`, and an interrupted seal completes on writer retry.
- Release/rollout: all native writers sharing one state directory must upgrade
  together. Stop/restart every old `serve` daemon and any queued or in-flight
  `agent-session notice` writer before the first new-binary write. Otherwise
  retired bytes can change, breaking native notices and Claude History until
  repair. Independent state directories can activate separately; existing open
  descriptors still require quiescence, and older binaries cannot read successors.

### Why / context

- A permanent global claim count eventually refused every native notice,
  including fresh sessions. Lifecycle transitions must free live capacity while
  preserving unknown operations, idempotency and exact History proofs.

### Evidence

- Three acceptance tests were RED before production changes: saturated finished
  ledger refused admission, archived keys could be reused, and archived proofs
  were omitted. Synthetic lifecycle fixtures now pass; live namespaces remain
  untouched.
- The older-writer append probe was RED before the read-only seal guard. Recovery
  fixtures also cover publication interrupted before sealing and unfinished
  claims finishing in the writable successor.

### Links

- <https://github.com/sympoies/nils-cli/issues/3330>
