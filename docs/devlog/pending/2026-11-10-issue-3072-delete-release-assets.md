## 2026-11-10 - Delete named release assets
<!-- devlog-id: 2026-11-10-issue-3072-delete-release-assets -->

### Result

- Added GitHub release asset deletion with preflight name and optional SHA-256
  checks.

### Why / context

- Release assets can be removed individually without changing the release or
  tag.

### Evidence

- Provider fixture tests cover refusal, dry run, digest guard and exact
  deletion.
