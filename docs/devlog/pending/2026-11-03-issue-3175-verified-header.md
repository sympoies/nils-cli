## 2026-11-03 - Bind Codex metrics header facts to the verified transcript
<!-- devlog-id: 2026-11-03-issue-3175-verified-header -->

### Result

- Codex metrics now read creation metadata through the same verified descriptor
  as retained-file counts, then rewind before counting. The shared provider
  header parser retains its existing format and byte bounds.

### Why / context

- An ancestor-directory exchange can preserve the original file fingerprint
  while a path reopen reads a substitute same-identity header. Descriptor-bound
  parsing keeps header-derived age and content counts in one snapshot.

### Evidence

- A deterministic synthetic regression first returned substitute creation year
  2025 with original-file counts, then passed with the original 2026 timestamp
  after the repair. No native provider occurrence is claimed.

### Links

- <https://github.com/sympoies/nils-cli/pull/3259>
