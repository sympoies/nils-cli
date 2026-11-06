## 2026-11-06 - Require canonical Claude transcript filenames
<!-- devlog-id: 2026-11-06-issue-3134-canonical-claude-filenames -->

### Result

- Claude prompt-source discovery uses only the exact conversation filename; the
  full-history Claude collector and its obsolete scan test are removed.

### Why / context

- Repository code, specs and provider-facing docs name no current
  producer/provider version for noncanonical filenames. Deleted-source resume
  already uses the exact filename contract, so the unnamed fallback has no
  documented external-system justification.

### Evidence

- The replacement rejection test ran before production edits and failed because
  matching metadata under another filename was discovered. Existing canonical
  discovery, safety and resume fixtures are retained.

### Links

- <https://github.com/sympoies/nils-cli/issues/3134>
