## 2026-10-27 - Speech fixture generation and ASR replay CLI
<!-- devlog-id: 2026-10-27-speech-test-fixture-replay -->

### Result

- Added a clap-based CLI that catalogs hash-pinned audio fixtures, generates
  through a configurable TTS adapter, records human reviews, replays against
  ASR endpoints, and scores saved runs offline.

### Why / context

- Explicit generation, request-fingerprint caching, and preflight review/hash
  checks keep repeated ASR evaluation separate from paid audio creation.

### Evidence

- 32 crate unit and contract tests passed; all HTTP calls used loopback stubs.
- `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast` passed, including
  the workspace Rust checks and environment isolation guard.

### Links

- <https://github.com/sympoies/nils-cli/issues/3001>
