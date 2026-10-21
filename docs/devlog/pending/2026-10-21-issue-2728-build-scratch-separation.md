## 2026-10-21 - Separate build scratch from evidence
<!-- devlog-id: 2026-10-21-issue-2728-build-scratch-separation -->

### Result

- `git-cli worktree` records a deterministic build directory under
  `AGENT_HOME/build/<repo-key>/<slug>` (`add` record, `go --build`) and
  reclaims it on `remove`.
- `agent-out` gains `project --kind scratch`, a run-scoped `build` allocation,
  per-class cleanup retention, and a build-tree flag for evidence runs.

### Why / context

- The agent write-location contract gives build output, scratch, and evidence
  separate roots and retention, so cleanup no longer trades early evidence loss
  against keeping multi-gigabyte build trees.

### Evidence

- RED then GREEN: the new worktree build-directory tests and the agent-out
  write-class tests fail on `origin/main` and pass with the change; clippy and
  the focused suites for the touched crates pass.
- Overlap with #2401 checked: that issue moves gate reports, while this change
  only names the worktree build directory.

### Links

- <https://github.com/sympoies/nils-cli/issues/2728>
- <https://github.com/sympoies/agent-runtime-kit/issues/302>

### Follow-ups

- The agent-out write-class code is an inline module in `cli.rs` and its tests
  live in `cleanup_retention.rs` because the artifact-routing hook refuses new
  files under `crates/agent-out`; move them into their own files once that is
  fixed.
