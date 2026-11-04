## 2026-11-04 - Label mailbox sender lineage and verification guidance
<!-- devlog-id: 2026-11-04-issue-3304-sender-relation -->

### Result

- Inbox and body views expose parent, ancestor or peer with neutral message
  classifications. Multiline wakes include relation labels and verification
  steps; bounded plain wakes direct recipients to the inbox.
- Inbox verification steps appear once in the header. Remote-encoded claims
  of local senders remain peers, without dispatcher wording.

### Why / context

- Blanket peer warnings obscured authorized dispatcher follow-ups after context
  compaction. Relation remains descriptive and does not change authentication,
  delivery or command admission.

### Evidence

- Five synthetic regressions executed before production edits and failed on
  absent relation fields or guidance. Focused and local-fast results are
  recorded in the delivery PR.
- A separate local-identity spoof regression failed before repair; direct wait
  coverage verifies body relation and full guidance.

### Links

- <https://github.com/sympoies/nils-cli/issues/3304>
