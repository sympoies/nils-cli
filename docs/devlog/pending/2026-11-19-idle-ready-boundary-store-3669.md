## 2026-11-19 - Add an internal idle-ready boundary store
<!-- devlog-id: 2026-11-19-idle-ready-boundary-store-3669 -->

### Result

- Added an internal readiness record that admits one consumption under the
  session record lock and retains the spent turn watermark across restart.

### Why / context

- Independent prompt request IDs can refer to the same idle period. The
  readiness record carries one shared grant while keyed receipts keep their
  request-specific replay semantics.

### Evidence

- The focused fixture tests cover concurrent consumption, intervening turn
  evidence, identity change and storage failure.

### Links

- <https://github.com/sympoies/nils-cli/issues/3669#issuecomment-6985116725>

### Follow-ups

- Wire the readiness record to the session projection, guarded prompt admission
  and keyed receipts.
