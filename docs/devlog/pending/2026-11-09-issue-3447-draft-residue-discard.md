## 2026-11-09 - Keep runtime residue out of saved drafts and support discard
<!-- devlog-id: 2026-11-09-issue-3447-draft-residue-discard -->

### Result

- Saved drafts exclude recognized runtime-owned notice residue.
  Owner-authenticated HTTP and local CLI discard remove one record and update
  or remove the stale-draft projection.

### Why / context

- Draft preservation protects user text; pending runtime delivery residue must
  not become a user draft. Discard belongs to the session across runtime
  incarnations and reports missing IDs explicitly. Projection persistence
  precedes deletion so a failed replacement write preserves the saved draft.

### Evidence

- Before production edits, the focused regression run failed four acceptance
  cases while user-text preservation passed. The affected draft/mailbox suite
  passed 26 tests and the CLI discard round trip passed. A retained failure
  regression also reproduced draft deletion after failed projection persistence
  before the ordering repair.

### Links

- <https://github.com/sympoies/nils-cli/issues/3447>
