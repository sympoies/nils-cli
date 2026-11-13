## 2026-11-13 - Exact webhook delivery selection
<!-- devlog-id: 2026-11-13-issue-3679-exact-deliveries -->

### Result

- Added explicit delivery ids to repository and organization redelivery
  commands and batch rows.

### Why / context

- Cutover recovery must select the recorded set exactly. Resolve all requested
  ids before any POST for that hook, reject mixed filters, and reuse the
  existing redelivery and metadata-only read-back path.

### Evidence

- Recorded four pre-implementation acceptance failures; all focused webhook
  tests now pass, including exact selection, pagination, unknown-id preflight
  and dry-run privacy.

### Links

- <https://github.com/sympoies/nils-cli/issues/3679>
