## 2026-11-26 - Exercise symlinked temporary paths in Codex fixtures
<!-- devlog-id: 2026-11-26-codex-skill-fixture-path -->

### Result

- Compare canonical skill and capsule paths in the affected tests without
  changing production behavior.

### Why / context

- Codex emits canonical paths even when a fixture is rooted at a symlinked
  temporary directory.

### Evidence

- Reproduced the raw-path assertions and exercised the Codex package with
  canonical and symlinked temporary roots.

### Links

- <https://github.com/sympoies/nils-cli/issues/4063>
