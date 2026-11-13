## 2026-11-13 - Send and mailbox design separates observation from attempt evidence
<!-- devlog-id: 2026-11-13-issue-3186-send-wake-design -->

### Result

- Added a proposed, source-pinned state/failure matrix and shared-boundary
  design; production behavior and tests remain unchanged.

### Why / context

- Recent fixes share render-loss and effect-confirmation boundaries, but
  scheduling, provenance, provider integration and command intent require
  distinct acceptance. Closed seed issues also carry different live-evidence
  dispositions.

### Evidence

- Inspected six weeks of merged history, current source owners and issue
  bodies/comments at main 35c22691c; no Rust build or live session probe.

### Links

- <https://github.com/sympoies/nils-cli/issues/3186>

### Follow-ups

- The lead chooses implementation slices after design review; pending native
  lifecycle and fault acceptance stays with its existing owner.
