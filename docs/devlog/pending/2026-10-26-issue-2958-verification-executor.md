## 2026-10-26 - Move verification executor into scripts/ci
<!-- devlog-id: 2026-10-26-issue-2958-verification-executor -->

### Result

- Moved the full and docs-only verification executor under scripts/ci and
  updated its entrypoints, resource-wrapper recursion, fixtures, and maintained
  documentation.
- Removed the retired verification skill and the redundant pre-PR wrapper.

### Why / context

- The verification sequence remains load-bearing for local, CI, and release
  validation, but its implementation no longer belongs in a project-local
  skill.

### Evidence

- The docs-only canonical entrypoint, resource admission tests, merge-plan
  tests, coverage-ratchet tests, local-fast tests, workflow contracts, and
  path-sensitive audits passed. The full canonical gate remains with CI.

### Links

- <https://github.com/sympoies/nils-cli/issues/2958>
