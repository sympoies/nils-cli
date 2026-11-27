## 2026-11-28 - Reopen pull requests and retrigger PR workflows
<!-- devlog-id: 2026-11-28-issue-4180-pr-reopen -->

### Result

- Added provider-neutral PR reopen with state read-back and documented how to
  retrigger PR workflows at an unchanged head.

### Why / context

- Allow a closed PR to be reopened and run the hosted probe at an unchanged
  head.

### Evidence

- Provider-mocked close and reopen contract tests for GitHub and GitLab.

### Links

- <https://github.com/sympoies/nils-cli/issues/4180>
