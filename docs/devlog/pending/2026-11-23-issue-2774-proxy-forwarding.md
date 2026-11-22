## 2026-11-23 - Avoid materializing Codex plug-in catalogs in proxy observations
<!-- devlog-id: 2026-11-23-issue-2774-proxy-forwarding -->

### Result

- The upstream forwarding observer reads only the authorization, bootstrap, and
  projection fields while the proxy relays the original WebSocket message.

### Why / context

- The catalog wire buffer was shared by the message clone. The complete decoded
  JSON tree dominated temporary allocations.

### Evidence

- A synthetic 16 MiB catalog replay through tungstenite retained the same
  receive buffer while observer allocations fell from 135,772,771 bytes to 640
  bytes.

### Links

- <https://github.com/sympoies/nils-cli/issues/2774>
