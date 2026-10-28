## 2026-10-28 - Guarded send proves captured Codex wrapped pastes
<!-- devlog-id: 2026-10-28-issue-2979-codex-wrapped-proof -->

### Result

- Codex single-line wrapped ASCII and CJK pastes can use the existing
  cursor-cell proof before Enter.

### Why / context

- Real 80-column captures omit word-wrap separators and a final ASCII space;
  cursor home and N-1/N endpoints must verify logical text that visible rows
  alone cannot prove.

### Evidence

- Both captured Codex regressions returned send-composer-unobservable before
  the implementation and passed after it; layout and cursor mismatches still
  refuse Enter.

### Links

- <https://github.com/sympoies/nils-cli/issues/2979>
