## 2026-10-29 - Align review handoff inspection with published approval admission
<!-- devlog-id: 2026-10-29-issue-3018-inspect-approval -->

### Result

- Active review handoff inspection exposes reviewed and the admitted native
  approval using the existing check gate, including finally-approved for an
  exact-head approval with an empty-findings owned observation.

### Why / context

- Lifecycle-only inspection previously reported awaiting-designated-review even
  when check accepted the same provider facts. Inspection remains read-only and
  preserves pending, lifecycle and fail-closed outcomes.

### Evidence

- A meaningful pre-fix regression failed on awaiting-designated-review versus
  reviewed; the affected handoff fixture suite then passed all 129 tests,
  including stale-head, strict source-only, wrong-author and unresolved-finding
  controls.

### Links

- <https://github.com/sympoies/nils-cli/issues/3018>
