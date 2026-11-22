## 2026-11-23 - Measure session provider restore duration
<!-- devlog-id: 2026-11-23-issue-3175-session-resume-metrics -->

### Result

- The existing authenticated session metrics endpoint reports current-runtime
  Claude and managed Codex restoration timing or a fixed unknown reason.

### Why / context

- Gate-bound monotonic timing distinguishes restoration from new starts and
  survives daemon reconnects without examining transcript content.

### Evidence

- Synthetic Claude and Codex acceptance tests failed on the uninstrumented
  baseline. Fixtures exercise the primary hook, initial proxy response,
  identity changes and timing limits.

### Links

- <https://github.com/sympoies/nils-cli/issues/3175>

### Follow-ups

- Validate released provider event and compaction semantics through the
  coordinator-scheduled native acceptance.
