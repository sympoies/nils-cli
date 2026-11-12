## 2026-11-12 - Test-change rows: module paths, retired form, JS titles
<!-- devlog-id: 2026-11-12-test-changes-row-keys -->

### Result

- Per-test deleted-test rows now include the enclosing Rust module path, the
  undeclared diagnostic names the retired file-level form and lists stale rows,
  and JavaScript titles on their own line or with escaped quotes are extracted
  with it.each/test.each tables detected as one row per table.

### Why / context

- Follow-ups F3, F4 and F5 from the per-test row change left same-named tests
  in different modules sharing one row, silent stale rows, and collapsed JS
  titles.

### Evidence

- Focused forge-cli test_integrity tests; failing fixtures recorded before the
  fix.

### Links

- Refs #3593
