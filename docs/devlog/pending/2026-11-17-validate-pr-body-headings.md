## 2026-11-17 - Validate configured PR body headings
<!-- devlog-id: 2026-11-17-validate-pr-body-headings -->

### Result

- PR body operations reject invalid or identical resolved section headings with
  key diagnostics.

### Why / context

- Field-by-field configuration layering and defaults can otherwise produce
  headings that cannot identify distinct sections.

### Evidence

- Focused heading regression tests exercised invalid values, layering, defaults
  and repair guidance.

### Links

- <https://github.com/sympoies/nils-cli/issues/2619>
