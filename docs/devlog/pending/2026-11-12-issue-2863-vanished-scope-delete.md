## 2026-11-12 - Accept a vanished session scope as a verified stop
<!-- devlog-id: 2026-11-12-issue-2863-vanished-scope-delete -->

### Result

- Deleting a session no longer refuses when the tmux kill ends every runtime
  process and the empty scope is removed before the whole-scope kill.

### Why / context

- The kill write failed on the removed scope and surfaced as an unverifiable
  termination, so the first delete was refused and only a retry succeeded.
  Foreign-user process permission errors were not the cause; they only appeared
  as inventory diagnostics.

### Evidence

- A real transient scope test removes the scope before termination; end-to-end
  delete runs on an isolated state directory and tmux socket succeed on the
  first attempt, and an escaped session process is still named and refused.

### Links

- <https://github.com/sympoies/nils-cli/issues/2863>
