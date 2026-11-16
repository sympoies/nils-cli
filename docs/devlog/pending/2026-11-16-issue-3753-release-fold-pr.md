## 2026-11-16 - Prepare reviewed devlog folds automatically after releases
<!-- devlog-id: 2026-11-16-issue-3753-release-fold-pr -->

### Result

- Release publication and manual recovery now prepare one UTC-dated devlog fold
  PR using the existing fold script and bot App credentials.

### Why / context

- A reviewed maintenance PR keeps immutable fragment transfers under the
  existing checks and merge queue while avoiding a missed manual release step.
  Dry-run and date-level PR preservation make recovery explicit.

### Evidence

- Eleven synthetic repository/API tests passed, covering dry-run, empty folds,
  new month files, exact API payloads, duplicate-date PRs and refusal of
  unverified publication.

### Links

- <https://github.com/sympoies/nils-cli/issues/3753>
