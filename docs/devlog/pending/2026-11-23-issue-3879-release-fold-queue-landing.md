## 2026-11-23 - Verify release folds after protected queue advancement
<!-- devlog-id: 2026-11-23-issue-3879-release-fold-queue-landing -->

### Result

- The post-publication fold replays the signed release bump onto the exact
  signed squash's parent and verifies the complete landed tree.

### Why / context

- An earlier protected queue entry can advance the release PR base or make
  an independent edit to a file the release bump also changes.

### Evidence

- Focused fixtures reproduced the queue parent mismatch and the
  clean same-file edit that changes both blob IDs. The full
  `devlog-fold-pr.test.cjs` suite rejected altered landing proofs.

### Links

- <https://github.com/sympoies/nils-cli/issues/3879>
