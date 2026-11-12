## 2026-11-12 - agent-session reports registry lock holder on timeout
<!-- devlog-id: 2026-11-12-agent-session-reports-registry-lock-holder-on-timeout-18e74f4b2feacfad-3d579d -->

### Result

- A registry-lock `coordination-lock-timeout` now reports the holder's operation class, pid and hold time in `details.holder`; a descriptor whose process is gone is reported as `stale` only.

### Why / context

- Carried over from #2977 so lock-holder diagnosis no longer relies on inference. Refs #3560.

### Evidence

- Five new `lock_holder_tests`; `--local-fast` passed (3077 passed, 3 skipped).
