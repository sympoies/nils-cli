## 2026-10-26 - Retire the final repository release skill
<!-- devlog-id: 2026-10-26-issue-2960-release-preparation -->

### Result

- Hosted release preparation invokes scripts/ci/prepare-release-version.sh
  directly, and the retained helper tests run from scripts/ci/tests without
  skill discovery.

### Why / context

- The release broker still needs the canonical version transform after
  repository-local skills retire; preserve the release tree, commit subject and
  checksum-bound artifact protocol.

### Evidence

- The direct invocation contract failed before relocation and passed afterward;
  retained helper, workflow, release-classifier and local-fast planner tests
  passed with bounded resources.

### Links

- <https://github.com/sympoies/nils-cli/issues/2960>
