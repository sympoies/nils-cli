## 2026-11-04 - Run the Claude release check hourly
<!-- devlog-id: 2026-11-04-issue-3275-hourly-release-check -->

### Result

- The scheduled Claude Code release check now runs hourly instead of daily.

### Why / context

- A daily schedule left a window of hours in which a new stable release could
  be installed by auto-updating hosts before a range-extension candidate
  existed. Default-branch-only runs, the concurrency group and the
  version-keyed candidate PR and issue handling are unchanged, so repeated
  runs for one version update nothing new.

### Evidence

- The release-check fixture tests pass, including repeated-run and duplicate
  handling for the candidate PR and failure issue. One real scheduled run is
  observed after merge.

### Links

- <https://github.com/sympoies/nils-cli/issues/3275>
