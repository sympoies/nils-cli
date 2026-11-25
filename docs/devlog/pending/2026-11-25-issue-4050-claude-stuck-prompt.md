## 2026-11-25 - Report stuck Claude structured prompts
<!-- devlog-id: 2026-11-25-issue-4050-claude-stuck-prompt -->

### Result

- Report a stuck non-placeholder Claude structured prompt as unconfirmed while
  preserving the composer.

### Why / context

- Terminal confirmation previously allowed a stuck short prompt to proceed as
  though delivery had succeeded.

### Evidence

- A synthetic short wake remains in the composer after Enter and receives a
  typed failure; queued prompt acknowledgement retains its existing tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/4050>
