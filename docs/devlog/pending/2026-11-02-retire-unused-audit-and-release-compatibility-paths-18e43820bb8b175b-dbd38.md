## 2026-11-02 - Retire unused audit and release compatibility paths
<!-- devlog-id: 2026-11-02-retire-unused-audit-and-release-compatibility-paths-18e43820bb8b175b-dbd38 -->

### Result

- Completion audits reject the obsolete strict flag, the local installer
  preserves existing files, and release validation keeps only its active
  helper.

### Why / context

- These paths no longer belong to the current repository contract; active
  callers migrate together.

### Evidence

- Focused completion, installer, release workflow and merge-plan fixtures
  passed through resource admission.

### Links

- <https://github.com/sympoies/nils-cli/issues/3075>
