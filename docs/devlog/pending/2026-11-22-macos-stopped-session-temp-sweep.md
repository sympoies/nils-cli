## 2026-11-22 - Reclaim proven stopped macOS session temp paths
<!-- devlog-id: 2026-11-22-macos-stopped-session-temp-sweep -->

### Result

- Temp sweep can reclaim a same-boot macOS runtime after exact broker
  revocation and tmux, pane, process group, and heartbeat ownership checks
  while preserving the session record.

### Why / context

- A missing process group alone stays unknown in shared coordination. Scoped
  terminal evidence lets the temp owner reclaim paths without weakening other
  readers.

### Evidence

- The Linux-hosted non-Linux regression reproduced removed 0 and retained
  unknown before the fix. The scheduled macOS baseline observed the same
  retention with the stopped record present.

### Links

- <https://github.com/sympoies/nils-cli/issues/3936>
