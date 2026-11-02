## 2026-11-02 - Share operator rules across session commands and serve
<!-- devlog-id: 2026-11-02-issue-3194-operator-rules -->

### Result

- Added one versioned JSON operator rules file, shared validation and
  effective-rule inspection, with the coordinator root requirement covering
  both coordinator roles at every entry path.

### Why / context

- Standalone CLI and hook commands do not inherit serve-only configuration.
  Invalid rules must keep commands available without admitting partial
  relaxations, while serve retains its last valid rules.

### Evidence

- Seven focused executable acceptance tests pass across CLI starts, daemon
  relay, HTTP create/resume, metadata updates, malformed-file mailbox
  availability, common checks and file resolution. Behavioral RED preceded
  implementation; file-size audit passes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3194>
