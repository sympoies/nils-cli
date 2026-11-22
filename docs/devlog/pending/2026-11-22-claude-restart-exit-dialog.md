## 2026-11-22 - Handle Claude exit dialogs during non-forced restart
<!-- devlog-id: 2026-11-22-claude-restart-exit-dialog -->

### Result

- Refuse visible background work before exit, cancel the task exit dialog, and
  discard unsent feedback before verifying provider exit.

### Why / context

- Claude can show an idle foreground prompt while tasks are running or pause
  its exit pending a decision.

### Evidence

- Synthetic pane fixtures reproduced the old exit timeout for background work
  and both dialogs.

### Links

- <https://github.com/sympoies/nils-cli/issues/3934>
