## 2026-11-05 - Prove Codex notice ownership across blank composer rows
<!-- devlog-id: 2026-11-05-issue-3354-codex-notice-composer -->

### Result

- Guarded Codex terminal notices recognize unchanged envelopes and prove
  logical cursor length and row boundaries before submission or owned cleanup;
  owner drafts remain preserved.

### Why / context

- Codex can render empty rows and typed blank spaces identically. Matching the
  notice display alone cannot authorize Enter or deletion, including when
  spaces move between rows without changing total length.

### Evidence

- A disposable Codex probe reproduced the post-paste mismatch. The corrected
  fixture failed on current main with the same receipt before production edits;
  all 16 focused notice tests passed after the fix, including Claude controls
  and draft-preservation cases.

### Links

- <https://github.com/sympoies/nils-cli/issues/3354>
