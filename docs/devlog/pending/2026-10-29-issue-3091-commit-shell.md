## 2026-10-29 - Clarify commit authoring shell contract
<!-- devlog-id: 2026-10-29-issue-3091-commit-shell -->

### Result

- Clarified the sole-command exception for semantic-commit authoring while
  retaining the development environment requirement for other shells.

### Why / context

- The delivery guard requires the authoring command to stand alone, so sourcing
  in that call conflicts with the guard.

### Evidence

- Issue acceptance specifies the docs-only validation lane and normal signing
  and hook enforcement.

### Links

- <https://github.com/sympoies/nils-cli/issues/3091>
