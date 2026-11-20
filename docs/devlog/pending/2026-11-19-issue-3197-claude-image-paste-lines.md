## 2026-11-19 - Preserve standalone image-path lines in staged Claude prompts
<!-- devlog-id: 2026-11-19-issue-3197-claude-image-paste-lines -->

### Result

- Preserve LF inside bracketed paste when a staged Claude prompt contains a
  line naming an existing image file alongside other text. Text-only prompts
  keep the existing CR separators.

### Why / context

- The default tmux separator turns an image path followed by text into one
  LF-delimited row.

### Evidence

- A private tmux receiver reproduced the CR byte and verified that tmux
  preserves LF with -r. The regression and its text-only control assert the
  exact bracketed-paste bytes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3197>
