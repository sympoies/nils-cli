## 2026-10-26 - Name split-cgroup termination refusals
<!-- devlog-id: 2026-10-26-issue-2982-cgroup-membership -->

### Result

- Session termination reports the process-session-member-cgroup check and
  explicit owner recovery when a captured member is outside the pane cgroup.

### Why / context

- Older launchers can leave helpers in the original cgroup while the pane
  enters a dedicated scope. The termination fence must remain closed; a generic
  verification error hides the required owner action.

### Evidence

- Disposable older-launcher fixture failed first on the missing check
  diagnostic; focused validation is recorded in the linked PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/2982>
