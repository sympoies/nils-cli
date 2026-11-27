## 2026-11-28 - Isolate workspace test fixtures
<!-- devlog-id: 2026-11-28-issue-4172-fixture-isolation -->

### Result

- Added a shared fixture for a private temporary directory, short Unix runtime
  root, and non-signing Git configuration. Added an audit for direct isolation
  bypasses in Rust tests and migrated the git-lock tag fixture.

### Why / context

- A fixture inherited command-scoped tag signing and failed before reaching the
  behavior under test.

### Evidence

- The git-lock overwrite prompt test failed under forced signing before the
  migration and passed with the same environment afterward.

### Links

- <https://github.com/sympoies/nils-cli/issues/4172>
