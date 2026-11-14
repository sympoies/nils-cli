## 2026-11-14 - Recover an exact running owner after capability file loss
<!-- devlog-id: 2026-11-14-issue-3707-owner-capability-recovery -->

### Result

- Linux owners can reissue an absent canonical capability or rebind a stopped
  predecessor through work-context set --if-absent; unsafe and foreign owners
  remain refused.
- Recovery remains pending until a post-restoration heartbeat is established;
  an unavailable heartbeat owner returns a typed retryable refusal.

### Why / context

- Capability-authenticated adoption could not repair loss of the capability
  itself. Recovery instead requires kernel ancestry, persisted pane birth and
  namespace evidence under lifecycle serialization; incomplete and non-Linux
  proofs remain refused.

### Evidence

- Both deterministic recovery tests returned exit 65 before production edits.
  Current kit aggregate policy admitted the recovery command for Codex and
  Claude while blocking ordinary mutation controls.

### Links

- <https://github.com/sympoies/nils-cli/issues/3707>
