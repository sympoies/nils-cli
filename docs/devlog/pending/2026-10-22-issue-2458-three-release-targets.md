## 2026-10-22 - Retire Intel macOS release artifacts
<!-- devlog-id: 2026-10-22-issue-2458-three-release-targets -->

### Result

- Release and cache matrices now build Linux x86_64/aarch64 and macOS aarch64
  only; supported-target documentation and workflow contracts agree.

### Why / context

- The supported release set excludes Intel macOS. Matrix equality alone did not
  prevent the retired target from returning to both workflows.

### Evidence

- The new exact-target contract failed against the old matrix and passed after
  removal; release-workflow-contract.test.sh passes in a clean environment.

### Links

- <https://github.com/sympoies/nils-cli/issues/2458>
