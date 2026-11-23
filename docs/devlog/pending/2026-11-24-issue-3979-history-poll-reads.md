## 2026-11-24 - Bound repeated History transcript and notice reads
<!-- devlog-id: 2026-11-24-issue-3979-history-poll-reads -->

### Result

- Reuse verified live notice proofs and bounded History transcript projections
  across unchanged polls.

### Why / context

- Both 1.42.3 and 1.42.4 repeatedly read the live notice segment per listed
  session and rescanned unchanged transcript windows.

### Evidence

- Private synthetic daemons read approximately 464 MB in 32 warmed History list
  polls on each release. Focused Nextest coverage verifies bounded repeated
  reads for both inputs.

### Links

- <https://github.com/sympoies/nils-cli/issues/3979#issuecomment-7109248858>
