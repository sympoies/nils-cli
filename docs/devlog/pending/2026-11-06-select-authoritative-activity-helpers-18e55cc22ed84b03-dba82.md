## 2026-11-06 - Select authoritative activity helpers
<!-- devlog-id: 2026-11-06-select-authoritative-activity-helpers-18e55cc22ed84b03-dba82 -->

### Result

- Activity dispatch and doctor select an explicit helper, approved root, or
  trusted release sibling.

### Why / context

- Avoid attributing activity to an unrelated PATH binary while preserving stale
  override diagnosis.

### Evidence

- The regression fixture reproduced selection of a PATH poison before the fix.

### Links

- <https://github.com/sympoies/nils-cli/issues/3377>
