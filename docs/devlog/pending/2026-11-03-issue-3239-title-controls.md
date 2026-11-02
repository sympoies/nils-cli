## 2026-11-03 - Authenticated session title controls
<!-- devlog-id: 2026-11-03-issue-3239-title-controls -->

### Result

- Managed sessions can pin a title or return it to generated topics with
  revision-fenced title set and auto commands.

### Why / context

- Title-only access reuses recorded launch parent/root and adopted-steward
  identities; a display role alone grants no access. Both revisions advance to
  fence pending inference and metadata edits, and auto clears structured user
  topic ownership.

### Evidence

- Acceptance tests were written first; the baseline run failed because the
  title command was absent. Focused CLI, automatic-retitle fixture and
  completion checks passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3239>
