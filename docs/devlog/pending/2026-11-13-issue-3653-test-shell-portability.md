## 2026-11-13 - Catch GNU-only shell in test stubs during PR validation
<!-- devlog-id: 2026-11-13-issue-3653-test-shell-portability -->

### Result

- The PR check entrypoint rejects known GNU-only shell forms in test sources
  and fixtures, with exact reviewed fallback and classifier-input exceptions.

### Why / context

- A GNU-only stat stub escaped Linux PR checks and failed macOS
  release-candidate validation; a cargo-free static check catches this class
  earlier without adding a macOS PR runner.

### Evidence

- The pre-3654 webhook fixture first failed the new entrypoint regression
  assertion; all fourteen synthetic audit tests now pass, including option
  spelling classes, current-tree and anchored-fallback checks.

### Links

- <https://github.com/sympoies/nils-cli/issues/3653>
