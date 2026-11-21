## 2026-11-21 - Scope post-release devlog folds to each publication
<!-- devlog-id: 2026-11-21-issue-3879-same-day-fold-isolation -->

### Result

- Production preparation and PR history use the verified release ID. Manual
  folds keep the UTC date branch and production keeps the publication date as
  its exclusive cutoff.

### Why / context

- Merged folds remain visible in provider history. Keying a production branch
  by UTC date caused a manual fold or a different release to block an otherwise
  valid publication.

### Evidence

- Both same-day regression fixtures reproduced the preflight refusal before the
  fix. The post-fix fold suite passed 33 tests, covering replay, exact PR
  identity, and fetched main readback.

### Links

- <https://github.com/sympoies/nils-cli/issues/3879>
