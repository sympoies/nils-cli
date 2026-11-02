## 2026-11-02 - Preserve stale composer drafts before guarded delivery
<!-- devlog-id: 2026-11-02-issue-3241-stale-drafts -->

### Result

- Guarded deliveries can save stale observed drafts in a bounded private
  session store before shared cleanup; authenticated list and byte-exact read
  routes expose saved drafts, and board metadata names the latest event.

### Why / context

- An abandoned draft must not indefinitely prevent pending input, while
  unreadable panes and newly edited drafts must continue to refuse delivery.
  The existing reader and guarded admission order remain the owners of those
  boundaries.

### Evidence

- A pre-implementation regression failed because pending guarded delivery did
  not create a draft observation; focused and affected validation are recorded
  with the PR.

### Links

- <https://github.com/sympoies/nils-cli/issues/3241>
