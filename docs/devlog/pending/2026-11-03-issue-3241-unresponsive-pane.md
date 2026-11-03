## 2026-11-03 - Report unresponsive panes after guarded terminal input
<!-- devlog-id: 2026-11-03-issue-3241-unresponsive-pane -->

### Result

- Add the unresponsive turn-state projection and metadata-only pane_response
  board event for guarded submit and shared clear actions.

### Why / context

- A waiting provider snapshot alone cannot prove its pane accepted input. The
  four-second default matches the existing submit observation budget, while
  exact reader evidence excludes busy and unobservable panes and a changed pane
  clears the status.

### Evidence

- Tests first reproduced waiting instead of unresponsive. Three new
  dual-provider behavioral tests and all 23 serve-config tests passed through
  the repository gate wrapper; directly affected validation is recorded in the
  PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3241>

### Follow-ups

- Coordinators own recovery, Console consumption, release and native
  acceptance; no recovery action is added.
