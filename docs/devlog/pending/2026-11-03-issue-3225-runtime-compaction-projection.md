## 2026-11-03 - Classified runtime commands and boundary-anchored compactions
<!-- devlog-id: 2026-11-03-issue-3225-runtime-compaction-projection -->

### Result

- Claude and Codex history, incremental rows and card previews share
  runtime_command and compaction classes with bounded exact text.

### Why / context

- A Claude compact boundary and its following summary are one event. Bounded
  adjacent lookup preserves the same anchor through paging and pending tails,
  while display classification excludes runtime actions from human prompt
  projections without touching send admission or input bytes.

### Evidence

- Repository-gated synthetic HTTP/class checks: 25 passed; runtime input byte
  invariance: 1 passed; provider history: 81 passed and 1 existing stress test
  ignored; provider prompt: 84 passed. The existing compact echo deduplication
  fixture passes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3225>
