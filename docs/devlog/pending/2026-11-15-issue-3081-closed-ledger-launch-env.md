## 2026-11-15 - Reject closed ledger records missing launch environment
<!-- devlog-id: 2026-11-15-issue-3081-closed-ledger-launch-env -->

### Result

- Closed-ledger reads reject a returned record missing launch_env with
  board-ledger-unavailable instead of synthesizing an empty map; current
  producer projections and retention remain unchanged. Remove only original N9
  audit row 8.

### Why / context

- The accepted producer cutoff and aggregate absence receipt permit retirement
  of this stored form. Rejection leaves retained history unchanged; the
  unobserved root remains a release prerequisite owned by the deployment owner.

### Evidence

- Retained test-first baseline and meaningful pre-edit RED: the missing-field
  regression failed with exit 101 because the reader returned a synthesized
  empty map. Current producer, repeat-read, cursor and retention coverage
  accompanies the reader change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3081>
