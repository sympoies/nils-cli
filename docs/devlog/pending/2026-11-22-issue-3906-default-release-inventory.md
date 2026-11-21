## 2026-11-22 - Omit the development template from default binary artifacts
<!-- devlog-id: 2026-11-22-issue-3906-default-release-inventory -->

### Result

- The GitHub release package step and the default local installer use the same
  binary inventory, which excludes cli-template.

### Why / context

- The template serves as a workspace reference. Homebrew installs the archive's
  bin directory and completion policy uses the full workspace inventory.

### Evidence

- The release and installer fixtures reproduced the old behavior before passing
  with the filter. Synthetic archive manifests for all three targets and a
  staged local install each changed from 44 to 43 executables, dropping only
  cli-template.

### Links

- <https://github.com/sympoies/nils-cli/issues/3906>
