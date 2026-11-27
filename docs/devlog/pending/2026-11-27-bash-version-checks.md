## 2026-11-27 - Reject unsupported Bash before local checks
<!-- devlog-id: 2026-11-27-bash-version-checks -->

### Result

- The checks entrypoint requires Bash 4.4 and selects its interpreter for
  nested scripts.

### Why / context

- Bash 3.2 can fail late on empty arrays under nounset after consuming gate
  time.

### Evidence

- A Bash 3.2 regression test verifies the early diagnostic and supported
  interpreter selection.

### Links

- <https://github.com/sympoies/nils-cli/issues/4145>
