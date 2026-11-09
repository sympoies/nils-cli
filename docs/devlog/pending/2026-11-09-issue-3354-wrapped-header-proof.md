## 2026-11-09 - Verify wrapped Codex notice headers in physical pane coordinates
<!-- devlog-id: 2026-11-09-issue-3354-wrapped-header-proof -->

### Result

- Guarded Codex notices recognize a soft-wrapped header, submit once and
  confirm the cleared composer; idle-conflict cleanup removes only proven
  paste.

### Why / context

- Codex 0.162.1 draws word-wrapped header rows independently, so tmux joined
  capture does not reconstruct the logical header. End/Home checks distinguish
  a same-length hard-newline edit before Enter or deletion.

### Evidence

- A native-layout fixture reproduced send-guard-observation-timeout at submit
  with pasted=true, submitted=false, cleared=false and
  remaining_input=unobservable before production edits.

### Links

- <https://github.com/sympoies/nils-cli/issues/3354>
