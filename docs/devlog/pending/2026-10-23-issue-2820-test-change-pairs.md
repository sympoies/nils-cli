## 2026-10-23 - Remove the test-change admission pair cap
<!-- devlog-id: 2026-10-23-issue-2820-test-change-pairs -->

### Result

- Test-change admission accepts more than 32 file/kind pairs when every finding
  has a concrete declaration and independent reviewer confirmation.

### Why / context

- Pair counts imposed a semantic split requirement despite complete, bounded
  comparison evidence.

### Evidence

- The new 50-pair fixture failed before the fix and passes afterward; all 19
  focused admission unit tests and five command-level integration tests pass
  under a 6 GiB memory cap with no swap.

### Links

- <https://github.com/sympoies/nils-cli/issues/2820>
