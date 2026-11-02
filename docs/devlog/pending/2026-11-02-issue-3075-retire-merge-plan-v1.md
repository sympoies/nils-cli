## 2026-11-02 - Retire the merge-plan v1 reader
<!-- devlog-id: 2026-11-02-issue-3075-retire-merge-plan-v1 -->

### Result

- The CI merge-plan consumer accepts only nils-cli.merge-plan.v2 and holds
  every other version; the v1 schema file is deleted and the merge-plan specs
  name v2 as the only plan contract.

### Why / context

- Producers on main emit only v2 and every open PR base launcher was checked to
  emit v2, so the v1 reader had no remaining producer; the v1 and v3 cases are
  now asserted rejected.

### Evidence

- merge-plan-metadata.test.py failed (ValueError not raised for v1) before the
  change and passes after; the merge-plan test family, own-compat audit (four
  rows removed) and docs checks pass.

### Links

- <https://github.com/sympoies/nils-cli/issues/3075>
