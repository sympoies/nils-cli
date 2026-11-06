## 2026-11-06 - Bind provider entrypoints to the selected toolchain
<!-- devlog-id: 2026-11-06-issue-3376-provider-toolchain -->

### Result

- Provider ingress, activity hooks and notification argv select the runtime
  toolchain at execution time.

### Why / context

- Bare executable names allowed a different PATH entry to run despite an
  explicitly selected session runtime.

### Evidence

- Isolated render-and-execute fixtures reproduced the conflicting PATH behavior
  and exercise selection, migration and notification restoration.

### Links

- <https://github.com/sympoies/nils-cli/issues/3376>
