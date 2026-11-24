## 2026-11-24 - Enable direct cross-host resume for managed provider sessions
<!-- devlog-id: 2026-11-24-issue-3981-native-resume-admission -->

### Result

- Admit verified native Codex and Claude moves with signed source fencing,
  checkout binding, and explicit no-writer recovery.

### Why / context

- Operators need to move a stopped conversation without copying credentials or
  allowing a destination writer before the proof is verified.

### Evidence

- 43 focused resume tests passed, including a native Claude fixture for staged
  history, wrong account, read-only preflight, one writer, and exact replay.

### Links

- <https://github.com/sympoies/nils-cli/issues/3981>
