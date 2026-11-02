## 2026-11-02 - Require current designated review records
<!-- devlog-id: 2026-11-02-issue-3079-current-review-inputs -->

### Result

- Designated publication requires a current interval marker in every
  generation; retained handoffs require base_ref and stored review snapshots
  require writer-emitted findings fields.

### Why / context

- Every ancestor is validated, so cutover depends on a read-only open-record
  inventory rather than ledger rewrites. Omitted source-pass policy remains
  current and digest-stable; final approval, optional state and self-run
  encodings retain their meaning.

### Evidence

- Six focused regressions reproduced acceptance of retired inputs before
  production edits. Synthetic fixtures now use current bindings and base
  fields; approval tests cover both policy encodings and record digests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3079>
