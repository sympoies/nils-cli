## 2026-11-26 - Specify Console move-aware session start
<!-- devlog-id: 2026-11-26-issue-4059-console-resume-move -->

### Result

- Documented the versioned Console relay contract for prepared moves without
  enabling the route.

### Why / context

- Only the destination can admit a writer. An unconfirmed outcome requires
  read-only reconciliation and never permits replay.

### Evidence

- Cross-checked the literal refusal set against the native move code and the
  approved source stop-and-retain design.

### Links

- <https://github.com/sympoies/nils-cli/issues/4059>
