## 2026-11-27 - Key forge-cli test removals from pinned source scopes
<!-- devlog-id: 2026-11-27-issue-3604-test-scopes -->

### Result

- Test-change admission distinguishes same-named tests using enclosing scopes
  in complete source images.

### Why / context

- An added test in one scope could previously cancel a removed test in another
  when their enclosing declarations were outside the diff hunks.

### Evidence

- Regression fixtures reproduced the cross-scope cancellation and
  string-literal false positive. Provider fixtures exercise preview and
  admission warning surfaces.

### Links

- <https://github.com/sympoies/nils-cli/issues/3604>
