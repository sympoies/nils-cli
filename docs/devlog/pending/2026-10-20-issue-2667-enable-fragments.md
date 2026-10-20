## 2026-10-20 - Enable isolated devlog entries and reviewed maintenance folding
<!-- devlog-id: 2026-10-20-issue-2667-enable-fragments -->

### Result

- Enabled fragment authoring and merge-base ownership checks for contributor,
  agent and CI paths. The maintainer prepares a dedicated reviewed fold PR
  after each release with `.agents/scripts/fold-devlog.sh`.

### Why / context

- Concurrent PRs shared the monthly insertion point, causing conflicts and
  repeated exact-head review. A dedicated maintenance PR provides one fold
  owner without changing the version-only release patch contract or writing
  directly to the default branch.

### Evidence

- The docs gate regression first accepted a month edit, then rejected it after
  policy wiring. Repository tests cover parallel fragment merges, immutable
  merged fragments, exact maintenance folding and repeated folding.

### Links

- <https://github.com/sympoies/nils-cli/issues/2667>
