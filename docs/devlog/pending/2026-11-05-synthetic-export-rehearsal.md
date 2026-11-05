## 2026-11-05 - Synthetic source export rehearsal
<!-- devlog-id: 2026-11-05-synthetic-export-rehearsal -->

### Result

- A disposable source comment provides a behavior-neutral code delta for export
  validation.

### Why / context

- Exercise the draft export and existing CI without changing runtime behavior;
  never merge this rehearsal.

### Evidence

- The source delta is one clearly labelled comment; runtime assertions are
  unchanged.
