## 2026-10-24 - Add exact non-executing broker recovery evidence
<!-- devlog-id: 2026-10-24-add-exact-non-executing-broker-recovery-evidence-18e161dba74de00a-99daf -->

### Result

- Added authenticated admission preparation and bounded item-scoped broker
  proofs for ordinary recovery, with exact terminal receipts and conservative
  unknown evidence.

### Why / context

- The producer and consumer contract now supports lost-reply recovery without
  blind admission replay while preserving older replay compatibility.

### Evidence

- Focused authenticated CLI contract tests passed after a failing-first run;
  registry bytes and unrelated records remain unchanged during inspection.

### Links

- <https://github.com/sympoies/nils-cli/issues/2608>
