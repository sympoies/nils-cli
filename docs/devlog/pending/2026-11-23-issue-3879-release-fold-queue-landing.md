## 2026-11-23 - Verify release folds after protected queue advancement
<!-- devlog-id: 2026-11-23-issue-3879-release-fold-queue-landing -->

### Result

- The post-publication fold verifies the exact signed release squash against
  its real parent and matches the full delta of the signed release bump.

### Why / context

- An earlier protected queue entry can land between the recorded release PR
  base and the release squash.

### Evidence

- The focused queue fixture reproduced the regression before the fix. The
  full `devlog-fold-pr.test.cjs` suite passed afterward and rejected altered
  landing proofs.

### Links

- <https://github.com/sympoies/nils-cli/issues/3879>
