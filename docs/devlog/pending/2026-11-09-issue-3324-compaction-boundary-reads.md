## 2026-11-09 - Bound compaction boundary reads during history paging
<!-- devlog-id: 2026-11-09-issue-3324-compaction-boundary-reads -->

### Result

- Read preceding compaction records in bounded reverse chunks and stop when the
  association resolves.

### Why / context

- Repeated full-window reads amplified I/O on transcripts with many compact
  echoes.

### Evidence

- Synthetic 30 MB paging measurement and existing history row-class tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3324>
