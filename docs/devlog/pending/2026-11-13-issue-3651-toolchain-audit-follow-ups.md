## 2026-11-13 - Toolchain pin audit follow-up limits and test coverage
<!-- devlog-id: 2026-11-13-issue-3651-toolchain-audit-follow-ups -->

### Result

- Rustup option operands are skipped before selecting a channel; regression
  tests pin the eight audit guards that survived the original mutation run.

### Why / context

- The audit is a text check: the root toolchain file is the only version
  source, and metadata-only cargo_deny jobs deliberately rely on the runner
  toolchain. The runbook now states both limits and their review
  responsibilities.

### Evidence

- Pinned rustup commands failed seven subcases before the fix. All eight
  reconstructed mutations survived the original suite and are killed by
  distinct behavior tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3651>
