## 2026-11-01 - Research engine with CLI and local HTTP entrypoints
<!-- devlog-id: 2026-11-01-issue-3161-research-engine -->

### Result

- Added provider-selectable search and brief, bounded public URL reading, a
  shared request/result/error contract, and separate cited facts and model
  assessment.

### Why / context

- Keep provider wire formats isolated while CLI and HTTP execute one bounded
  engine; unknown usage and unread content must remain unknown.

### Evidence

- 18 focused offline tests passed. Package clippy and directly affected schema,
  completion, docs, file-size and publish-order checks were run. Live probes
  skipped for missing keys. Canonical publish dry-run is blocked by unpublished
  workspace dependency versions.

### Links

- <https://github.com/sympoies/nils-cli/issues/3161>
