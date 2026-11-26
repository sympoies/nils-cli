## 2026-11-26 - Declare generated fold PR test changes as None
<!-- devlog-id: 2026-11-26-issue-3863-fold-pr-none-declaration -->

### Result

- The generated devlog fold PR body now declares `## Test changes` as `None.`
  and states in `## Test plan` that it touches no test source.

### Why / context

- The review handoff requires the canonical no-change declaration; the section
  must be exactly `None.`, so the explanation lives in the next section.

### Evidence

- The exact generated body passes the test-change admission with an empty diff;
  the fold PR fixtures assert the new text.

### Links

- <https://github.com/sympoies/nils-cli/issues/3863>
