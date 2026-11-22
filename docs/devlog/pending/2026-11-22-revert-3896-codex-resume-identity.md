## 2026-11-22 - Revert the Codex resume identity retention before v1.42.0
<!-- devlog-id: 2026-11-22-revert-3896-codex-resume-identity -->

### Result

- The revert restores the Codex resume behavior from before #3896 and preserves
  the earlier devlog fragment.

### Why / context

- The release pull request's merge group failed its macOS partition on this
  change, and the change landed after the approved release candidate.
- The change returns later together with its macOS repair and hosted macOS proof.

### Evidence

- In macOS merge group 44967857004, the capture-timeout and write-failure tests
  each failed twice because the diagnose summary lacked the expected warning.
- The restored helper behavior is extracted into existing module paths to meet
  the file-size ratchet.

### Links

- <https://github.com/sympoies/nils-cli/issues/3883>
- <https://github.com/sympoies/nils-cli/pull/3896>
- <https://github.com/sympoies/nils-cli/actions/runs/44967857004>
