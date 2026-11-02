## 2026-11-02 - Add a public boundary audit for credentials and home paths
<!-- devlog-id: 2026-11-02-issue-3189-public-boundary-audit -->

### Result

- Added scripts/ci/public-boundary-audit.py: it fails on committed credential
  shapes and absolute user-home paths in tracked files, and runs in the
  docs-only, scoped, local-fast, and full check chains.
- Intended exceptions are exact paths and placeholder users in
  scripts/ci/public-boundary-allowlist.tsv, each with a reason; the audit holds
  no private word list.

### Why / context

- Public content must stay generic, and nothing stopped committed home paths or
  token-shaped values from re-entering fixtures and docs.

### Evidence

- python3 scripts/ci/tests/public-boundary-audit.test.py, plus the
  merge-plan-ci, devlog-policy, and local-fast CI tests, pass; the audit passes
  on the tree.

### Links

- <https://github.com/sympoies/nils-cli/issues/3189>
