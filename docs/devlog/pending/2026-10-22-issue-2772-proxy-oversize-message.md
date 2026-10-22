## 2026-10-22 - agent-session proxy survives oversized app-server messages
<!-- devlog-id: 2026-10-22-issue-2772-proxy-oversize-message -->

### Result

- The Codex app-server proxy now allows messages up to 128 MiB and answers an
  app-server message above 64 MiB with a per-request JSON-RPC error instead of
  exiting.
- A proxy that exits while the TUI is still running records its final error
  line in the session's startup diagnostic file.

### Why / context

- The remote plugin catalog response grew past the old 16 MiB cap, so new
  sessions lost their proxy about six seconds after the TUI connected, with no
  retained reason.

### Evidence

- Proxy tests relay a 17 MiB response, fail only the request behind a 65 MiB
  response, and check the recorded exit line.

### Links

- sympoies/nils-cli#2772

### Follow-ups

- A message above the 128 MiB WebSocket ceiling still ends the proxy; the exit
  line now says so.
