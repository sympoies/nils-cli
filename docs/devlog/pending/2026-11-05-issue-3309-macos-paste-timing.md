## 2026-11-05 - agent-session: allow bounded transport latency in owned paste proof
<!-- devlog-id: 2026-11-05-issue-3309-macos-paste-timing -->

### Result

- Claude terminal confirmation and owned opaque-paste cleanup use the existing
  fifteen-second terminal bound, with two seconds for each stable cursor
  position; malformed notice fixtures await server rejection.

### Why / context

- macOS release tests exposed sequential subprocess proof costs that short
  aggregate and settle windows did not cover. Keeping the malformed socket peer
  alive isolates protocol validity from Unix socket reset timing without
  weakening refusal assertions.

### Evidence

- Before production edits, two Linux regressions with 80 ms transport latency
  failed twice. After the fix, all five Console collapsed-paste tests and
  sixteen guarded-send placeholder and notice-refusal tests passed through the
  repository gate; the strict file-size audit passed. macOS verification
  remains the release merge group.

### Links

- <https://github.com/sympoies/nils-cli/issues/3309>
- <https://github.com/sympoies/nils-cli/issues/3330>
