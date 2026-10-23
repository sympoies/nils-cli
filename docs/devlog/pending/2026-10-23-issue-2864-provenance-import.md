## 2026-10-23 - Admit large imports through verified upstream provenance
<!-- devlog-id: 2026-10-23-issue-2864-provenance-import -->

### Result

- Designated review can admit complete large initial imports by verifying
  mapped added-file blob identity against a pinned upstream source and binding
  the verified manifest into independent confirmation.

### Why / context

- The ordinary 4 MiB and 300-file preview ceilings cannot represent a cohesive
  initial import; provenance keeps complete provider enumeration and bounded
  adaptation inspection without a bypass.

### Evidence

- Regression-first fixtures reproduced the large-import limit before
  implementation. All 28 focused test-integrity unit tests and 111
  review-handoff integration tests pass, including a 625-path fixture above 7.2
  MB.

### Links

- <https://github.com/sympoies/nils-cli/issues/2864>
