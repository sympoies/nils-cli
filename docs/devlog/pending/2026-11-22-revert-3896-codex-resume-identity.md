## 2026-11-22 - Revert the Codex resume identity retention before v1.42.0
<!-- devlog-id: 2026-11-22-revert-3896-codex-resume-identity -->

### Result

- The retention of the managed Codex resume identity (#3896) is reverted on main;
  the earlier devlog fragment stays as history because merged fragments cannot
  change in a pull request.

### Why / context

- The release pull request's merge group failed its macOS partition on this
  change, and the change landed after the approved release candidate.
- The change returns later together with its macOS repair and hosted macOS proof.

### Evidence

- The revert is the exact inverse of the original commit for the agent-hook crate
  and the agent-session activity sources.

### Links

- <https://github.com/sympoies/nils-cli/issues/3883>
- <https://github.com/sympoies/nils-cli/pull/3896>
