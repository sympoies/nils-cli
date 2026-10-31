## 2026-10-31 - Prepare isolated Claude range extension drafts
<!-- devlog-id: 2026-10-31-claude-release-preparation-3138 -->

### Result

- Added default-branch stable discovery, registry integrity verification,
  isolated module contract checking and immutable version-keyed draft/failure
  records without changing the current supported maximum.

### Why / context

- Contract checks alone cannot establish native delivery; each candidate
  retains native acceptance PENDING and exact-version native and reviewed merge
  gates. Immutable first evidence and candidate refs preserve human edits and
  review decisions across retries.

### Evidence

- Fixture acceptance covers discovery, integrity, child isolation, in-range
  no-op, passing drafts, failures, unavailable creation, deduplication and
  ownership refusals. An immediate-next-version replacement failure was
  observed before its repair.

### Links

- <https://github.com/sympoies/nils-cli/issues/3138>

### Follow-ups

- Exact-version native acceptance remains required for each future
  range-extension merge; no native success is claimed here.
