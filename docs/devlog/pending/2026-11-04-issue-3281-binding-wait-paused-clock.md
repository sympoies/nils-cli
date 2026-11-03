## 2026-11-04 - Run the initial-binding wait test on a paused clock
<!-- devlog-id: 2026-11-04-issue-3281-binding-wait-paused-clock -->

### Result

- The initial-binding wait reads the tokio clock, and its typed-cause test
  pauses time after the proxy is up, so the 30 s deadline elapses without real
  waiting (30.2 s to 0.4 s).

### Why / context

- The test waited the real control-submit deadline to prove the bounded
  refusal.

### Evidence

- fresh_thread_start_refuses_unresolved_binding_while_create_owns_record_lock:
  30.2 s before, 0.4 s after; assertions unchanged.

### Links

- <https://github.com/sympoies/nils-cli/issues/3281>
