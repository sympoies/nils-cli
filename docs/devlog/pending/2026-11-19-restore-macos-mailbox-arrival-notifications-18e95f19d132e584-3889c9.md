## 2026-11-19 - Restore macOS mailbox arrival notifications
<!-- devlog-id: 2026-11-19-restore-macos-mailbox-arrival-notifications-18e95f19d132e584-3889c9 -->

### Result

- Mailbox wait matches notifications against the canonical watched directory
  and relevant pull requests run the macOS arrival suite.

### Why / context

- The notification backend emits physical paths, while a symlinked state path
  could leave the registry filter comparing a different spelling; the shared
  wait budget exposed the missed event.

### Evidence

- MacOS baseline run 44473317490 reproduced both arrival timeouts before the
  fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3831>
