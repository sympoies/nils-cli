## 2026-11-25 - Bound admission liveness scans
<!-- devlog-id: 2026-11-25-issue-4037-admission-liveness -->

### Result

- Admission counts live tmux sessions from one bounded listing and reports
  unknown liveness in capacity refusals.

### Why / context

- Sequential probes could exhaust the shared liveness budget and count stopped
  records as active.

### Evidence

- A delayed synthetic tmux regression reproduces the pre-fix refusal and passes
  with the single scan; failed and timed-out listings remain conservative.

### Links

- <https://github.com/sympoies/nils-cli/issues/4037>
