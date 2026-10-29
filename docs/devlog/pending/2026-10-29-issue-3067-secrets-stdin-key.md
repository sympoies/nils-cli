## 2026-10-29 - Added atomic stdin single-key encrypted writes
<!-- devlog-id: 2026-10-29-issue-3067-secrets-stdin-key -->

### Result

- Added secrets set with exact stdin framing, ciphertext validation and atomic
  installation in a governed managed store worktree.

### Why / context

- Encrypted-write completion and signed commit/PR delivery are separate
  outcomes: set hands off delivery as pending without reusing add direct push
  behavior. Final-path SOPS rules override inherited recipient/config settings.

### Evidence

- Pre-edit CLI RED; disposable real SOPS stdin and managed-worktree probes;
  focused synthetic setter boundary tests.

### Links

- <https://github.com/sympoies/nils-cli/issues/3067>
