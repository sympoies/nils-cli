## 2026-11-20 - Preserve literal paths in idle-ready boundary prompts
<!-- devlog-id: 2026-11-20-issue-3669-idle-boundary-command-classifier -->

### Result

- Reuse the owner-command classifier so absolute paths and namespaced slash
  tokens enter the guarded prompt route as literal text.

### Why / context

- A leading slash alone matched both owner commands and ordinary prompt text.

### Evidence

- A route fixture verifies literal delivery, durable receipts and zero terminal
  input for a refused owner command.

### Links

- <https://github.com/sympoies/nils-cli/issues/3669>
