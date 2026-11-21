## 2026-11-19 - Fence queued prompts with an idle-ready boundary
<!-- devlog-id: 2026-11-19-queued-prompt-idle-boundary-3669 -->

### Result

- Published verified idle-ready boundaries on the session list and bound keyed
  guarded prompts to one durable consumption.

### Why / context

- Independent request keys protect replay of one attempt. A separate runtime
  boundary makes competing queue consumers share the same idle admission.

### Evidence

- Counting fixtures cover one native dispatch, stale snapshots, replacement,
  keyed replay, lost acknowledgement, terminal composer refusal, and a
  nonblocking session list.

### Links

- <https://github.com/sympoies/nils-cli/issues/3669#issuecomment-6985116725>
