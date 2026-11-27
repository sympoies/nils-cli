## 2026-11-28 - Guard idle retirement for managed Claude sessions
<!-- devlog-id: 2026-11-28-claude-idle-retirement -->

### Result

- Conditional idle deletion checks Claude hook activity, fresh native status
  and an empty composer under the existing session lock.

### Why / context

- An idle Claude runtime must be retired without losing a draft or interrupting
  provider work.

### Evidence

- The before-fix Claude fixture returned session-idle-delete-unsupported.

### Links

- <https://github.com/sympoies/nils-cli/issues/4181>
