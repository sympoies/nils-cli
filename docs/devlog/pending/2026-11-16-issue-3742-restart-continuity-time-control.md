## 2026-11-16 - Make restart continuity fixtures deterministic under load
<!-- devlog-id: 2026-11-16-issue-3742-restart-continuity-time-control -->

### Result

- Synthetic restart and continuity tests use scoped logical time and lifecycle
  events; production timing is unchanged.
- The read-side continuity test independently rejects a valid seventeen-link
  chain and accepts its sixteen-link suffix.

### Why / context

- Single-CPU contention made wall-clock polls and shell capture deadlines race
  the behavior under test.

### Evidence

- The baseline stress passed zero of thirty-two copies; workspace-selected
  continuity and JSON assertions pass with the test control.

### Links

- <https://github.com/sympoies/nils-cli/issues/3742>
