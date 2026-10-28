## 2026-10-29 - Reuse shared usage observations in offline cached diagnostics
<!-- devlog-id: 2026-10-29-issue-3050-offline-cached-observation -->

### Result

- Explicit cached Claude diagnostics import eligible shared account
  observations for previously unseeded aliases without HTTP requests.

### Why / context

- Live fallback already reused the shared snapshot, while offline diagnostics
  read only a target projection. The repair reuses that importer and retains
  original observation time, ownership, TTL and cooldown semantics.

### Evidence

- Loopback CLI regressions reproduced missing text cache and absent async JSON
  windows before the repair; both passed afterward with original timestamp and
  zero offline requests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3050>

### Follow-ups

- Keep the issue open until this specific repair is released.
