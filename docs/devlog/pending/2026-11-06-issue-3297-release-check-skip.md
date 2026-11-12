## 2026-11-06 - Skip repeated work in the hourly Claude release check
<!-- devlog-id: 2026-11-06-issue-3297-release-check-skip -->

### Result

- A scheduled Claude release check now skips the Rust setup, build, isolated
  install and provider writes when the newest version already has a candidate
  PR or a failure issue. Manual dispatch always runs the full check.

### Why / context

- While a candidate waited for native acceptance, each hourly run repeated the
  expensive steps, and a failing candidate turned every run red. The first
  failure still fails red and opens the version-keyed issue; later scheduled
  runs skip. An orphan candidate branch stays a maintainer reconciliation step,
  documented in the delivery and releases runbook.

### Evidence

- Fixture tests cover an existing candidate PR, an existing failure issue, a
  new version, an in-range version, an orphan branch and the workflow gating.

### Links

- <https://github.com/sympoies/nils-cli/issues/3297>
