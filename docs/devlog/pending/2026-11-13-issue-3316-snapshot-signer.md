## 2026-11-13 - Explain unavailable worktree snapshot signers
<!-- devlog-id: 2026-11-13-issue-3316-snapshot-signer -->

### Result

- Worktree backup signing refusals identify a configured full fingerprint and
  show principal selection plus identity doctor recovery.

### Why / context

- Principal snapshot authorization already worked; unselected callers with
  unavailable default keys only received a generic signing-setup hint.
  Arbitrary key selectors stay redacted.

### Evidence

- Synthetic-GPG regression failed before the fix; 222 focused worktree tests
  and the final five backup-signing tests passed with rustc 1.99.1.

### Links

- <https://github.com/sympoies/nils-cli/issues/3316>
