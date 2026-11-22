## 2026-11-22 - Restore Codex resume identity after the release queue revert
<!-- devlog-id: 2026-11-22-issue-3883-codex-resume-reland -->

### Result

- Restore exact Codex resume after normal exit and report unconfirmed capture
  through the typed hook warning and diagnose summary.

### Why / context

- The release-queue fixture passed a temporary path with a symlinked ancestor.
  The observation spool requires a symlink-free state path.

### Evidence

- Both warning cases failed at the diagnose assertion on hosted macOS. A
  symlinked Linux temp root reproduced both failures, and the canonical fixture
  state path restored all three lifecycle cases in debug and optimized builds.

### Links

- <https://github.com/sympoies/nils-cli/issues/3883>
- <https://github.com/sympoies/nils-cli/actions/runs/44967857004>
