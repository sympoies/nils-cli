## 2026-11-01 - Share Codex history discovery within rich CLI list queries
<!-- devlog-id: 2026-11-01-issue-3172-batched-list-discovery -->

### Result

- CLI list shares one bounded Codex history-header walk per equivalent provider
  root, effective configuration and launch profile. Each selected source still
  passes existing identity and replacement checks.

### Why / context

- Cold list processes repeatedly scanned every history header once per session.
  Request-local work sharing removes that amplification while preserving
  arbitrary filenames, unique matching and unknown results after truncation.
  Single-record, serve, board and Claude discovery lifetimes are retained.

### Evidence

- Frozen pre-production default-list IO oracle failed with 25362 openat and
  25189 read calls for 25 sessions and 1000 histories, then passed unchanged
  limits with 1266 openat and 1190 read calls. Complete rich JSON matched.
  Focused tests and affected-package all-targets Clippy passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3172>
