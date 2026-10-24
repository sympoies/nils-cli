## 2026-10-25 - Portable active-session admission at the persistence boundary
<!-- devlog-id: 2026-10-25-issue-2921-session-admission -->

### Result

- Optional state-root max_active_sessions atomically admits fresh starts and
  stopped-record reactivation; capacity refusals precede runtime launch and
  diagnose exposes read-only occupancy.

### Why / context

- The session record and its existing lifecycle lock serve as a reservation
  shared by CLI and HTTP callers, so cancellation and daemon restart need
  neither a separate counter nor an expiry mechanism. Unknown liveness
  conservatively keeps capacity occupied.

### Evidence

- Seven focused CLI/HTTP boundary tests pass. Retained pre-fix RED showed both
  concurrent creates succeeding at limit one and a stopped-record resume
  bypassing occupied capacity.

### Links

- <https://github.com/sympoies/nils-cli/issues/2921>
