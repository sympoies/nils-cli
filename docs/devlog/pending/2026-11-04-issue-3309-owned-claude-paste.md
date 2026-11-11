## 2026-11-04 - agent-session: bind collapsed Claude paste ownership to one send
<!-- devlog-id: 2026-11-04-issue-3309-owned-claude-paste -->

### Result

- Existing-session Claude sends submit or clear their own opaque paste only
  after empty admission and a verified atomic-token cursor proof.
  Admission-time pastes and owner suffixes remain refused.

### Why / context

- An opaque paste cannot prove its payload by text equality. Send-local
  ownership plus a logical-end proof distinguishes the newly pasted block from
  owner input, including trailing spaces; it is never carried into another send
  or startup admission.

### Evidence

- Before production edits, three Console regressions failed for stranded
  placeholders and mixed owner input. The gate-wrapped focused run then passed
  15 tests, including transient reads and late owner edits. A resident tester
  observed the atomic C-a, Right and Backspace model on Claude Code 2.1.317 in
  one disposable session.

### Links

- <https://github.com/sympoies/nils-cli/issues/3309>
