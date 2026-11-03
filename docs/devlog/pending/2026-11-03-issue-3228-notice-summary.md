## 2026-11-03 - Share notice summary labels and preserve mailbox wake bytes
<!-- devlog-id: 2026-11-03-issue-3228-notice-summary -->

### Result

- The notice command renders an optional validated 1..70-scalar summary from
  one shared kind-label table, retaining exact detail bytes and mailbox count
  envelopes.

### Why / context

- The history recognizer will consume the same label vocabulary. External
  body-only sender integration is bounded by a release-coordinator-owned
  2026-11-30 review and 2026-12-31 expiry.

### Evidence

- Tests-first RED captured unknown --summary before implementation.
  Repository-gated notice transport tests passed, including every merged
  fixture label, unknown kind, scalar-boundary rendering and unchanged
  body-only bytes.

### Links

- <https://github.com/sympoies/nils-cli/issues/3228>
