## 2026-11-28 - Gate agent-session temporary-directory leaks on PR heads
<!-- devlog-id: 2026-11-28-issue-4170-agent-session-tempdir-probe -->

### Result

- GitHub Actions runs the agent-session temporary-directory probe with zero
  retries when a PR changes the crate, a workspace path dependency, or a probe
  input.

### Why / context

- A hosted check lets reviewers verify the probe against the current PR head
  without waiting for a local probe slot.

### Evidence

- Actionlint, workflow filter cases against Git history, and the
  temporary-directory probe shell fixture passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/4170>
