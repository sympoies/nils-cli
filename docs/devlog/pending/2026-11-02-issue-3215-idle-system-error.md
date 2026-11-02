## 2026-11-02 - Admit a guarded send to a Codex thread left in system error
<!-- devlog-id: 2026-11-02-issue-3215-idle-system-error -->

### Result

- A guarded send or notice to a live Codex session now admits a thread whose
  provider status is systemError, which the provider reports for a loaded
  thread that is not running and has no pending request after an error event,
  such as a usage-limit stop. A thread that is not loaded, has no status or
  reports an unknown status is still refused with send-idle-unobservable.

### Why / context

- Refs 3215 and 3186: the guarded idle check accepted only idle, so a session
  that looked idle but kept the error status refused every send until its next
  turn started, and a bound-account change did not clear it.

### Evidence

- RED before the change: the new test reproduced send-idle-unobservable with
  the message native thread idle or identity could not be verified; the two
  negative controls passed. GREEN after: all 42 native guarded send tests pass.

### Follow-ups

- The status was read from the provider source, not from the affected live
  thread. The rebind probe and the startup quiescence check still require the
  idle status and are not changed here.
