## 2026-11-14 - Prepare closed-ledger retirement counts
<!-- devlog-id: 2026-11-14-issue-3081-closed-ledger-inventory -->

### Result

- Added an operator-run read-only aggregate closed-ledger probe and the
  deployment-owner count definitions for the remaining state-retirement slices.

### Why / context

- Board projection synthesizes missing launch metadata and can maintain the
  ledger, so it cannot establish old-record absence. Production retirement
  requires a separate deployment-owner version and cutoff receipt.

### Evidence

- Twelve synthetic CLI tests pass, including retention cutoffs, unknown status,
  privacy, unchanged ledger bytes and modification metadata, a held writer lock,
  and runtime-invalid encoding rejection. The encoding regression was retained
  RED before the one-line parser fix.
- Designated review F1 exposed duplicate typed fields and nonstandard constants
  falsely certifying counts. Five malformed JSON cases failed before the parser
  repair; the final suite rejects them while preserving record map semantics.

### Links

- <https://github.com/sympoies/nils-cli/issues/3081>
