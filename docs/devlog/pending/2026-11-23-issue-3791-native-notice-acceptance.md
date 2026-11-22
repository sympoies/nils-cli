## 2026-11-23 - Add disposable native notice acceptance for Claude and Codex
<!-- devlog-id: 2026-11-23-issue-3791-native-notice-acceptance -->

### Result

- Added a pinned, credential-free installed-binary profile that checks native
  notice classification and a typed lookalike on both providers.

### Why / context

- Consumer fixtures can now exercise the provider-owned notice and transcript
  paths with a synthetic loopback model response.

### Evidence

- Pinned provider probes exercised native delivery and verified deletion
  through the installed CLI in a private process namespace.

### Links

- <https://github.com/sympoies/nils-cli/issues/3791>
