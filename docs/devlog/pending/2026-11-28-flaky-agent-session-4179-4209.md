## 2026-11-28 - Make the startup-idle and stale-draft admission tests independent of host speed
<!-- devlog-id: 2026-11-28-flaky-agent-session-4179-4209 -->

### Result

- The Codex startup test freezes its clock at the runtime's recorded start, and
  the stale-draft admission test gives its direct branch the same cleanup
  budget as its guarded branch.

### Why / context

- A stall between creating the session and freezing the clock, or a slow
  per-call tmux fixture, moved the observed result without any product change.

### Evidence

- With a 1.2 s stall injected before the freeze, the old startup test read
  Unknown instead of Starting. With 0.25 s injected per fake tmux call, the old
  direct-branch cleanup exceeded its 2.5 s test budget and refused with
  composer-not-empty. Both pass 20 of 20 with the same injection after the
  change.

### Links

- <https://github.com/sympoies/nils-cli/issues/4179>
- <https://github.com/sympoies/nils-cli/issues/4209>

### Follow-ups

- Under whole-host CPU saturation a process spawn can exceed the 1 s
  per-command pane-capture cap; that product bound is unchanged here.
