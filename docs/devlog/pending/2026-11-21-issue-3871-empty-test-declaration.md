## 2026-11-21 - Accept empty test change declarations
<!-- devlog-id: 2026-11-21-issue-3871-empty-test-declaration -->

### Result

- Accept standalone None. as an empty test changes declaration while requiring
  rows for detected findings.

### Why / context

- A no-change declaration should be explicit without weakening review
  admission.

### Evidence

- Focused test integrity suite and local repository checks.

### Links

- <https://github.com/sympoies/nils-cli/issues/3871>
