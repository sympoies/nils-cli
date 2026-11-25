## 2026-11-25 - Diagnose capacity held by failed closes
<!-- devlog-id: 2026-11-25-diagnose-failed-close-slots -->

### Result

- Diagnose lists occupied sessions whose latest close for the current runtime
  found surviving processes.

### Why / context

- Operators need to identify which failed closes still contribute to session
  admission occupancy.

### Evidence

- Synthetic fixtures cover the current and superseded runtime generations,
  missing journals, successful later closes, and no journal writes.

### Links

- <https://github.com/sympoies/nils-cli/issues/4038>
