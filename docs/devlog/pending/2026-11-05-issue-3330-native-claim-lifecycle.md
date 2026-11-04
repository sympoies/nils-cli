## 2026-11-05 - Archive finished native notice claims without losing attestations
<!-- devlog-id: 2026-11-05-issue-3330-native-claim-lifecycle -->

### Result

- The native store caps live claims, seals eligible finished records into
  hash-linked immutable segments and keeps permanent compact key indexes.
- All hosts must upgrade agent-session together and quiesce older writers before
  activating this layout; older binaries cannot read or write its successors.

### Why / context

- A permanent global claim count eventually refused every native notice,
  including fresh sessions. Lifecycle transitions must free live capacity while
  preserving unknown operations, idempotency and exact History proofs.

### Evidence

- Three acceptance tests were RED before production changes: saturated finished
  ledger refused admission, archived keys could be reused, and archived proofs
  were omitted. Synthetic lifecycle fixtures now pass; live namespaces remain
  untouched.

### Links

- <https://github.com/sympoies/nils-cli/issues/3330>
