## 2026-11-22 - Fix local-fast empty selection on Bash 3.2
<!-- devlog-id: 2026-11-22-bash32-empty-selection -->

### Result

- Allow empty local-fast arrays under Bash 3.2 nounset.

### Why / context

- The platform Bash runtime failed before completing an empty selection.

### Evidence

- Run the local-fast entry point with an empty selection and the platform Bash
  regression check.

### Links

- <https://github.com/sympoies/nils-cli/issues/2903>
