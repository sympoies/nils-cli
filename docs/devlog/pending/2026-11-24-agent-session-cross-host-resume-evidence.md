## 2026-11-24 - Define signed cross-host resume evidence
<!-- devlog-id: 2026-11-24-agent-session-cross-host-resume-evidence -->

### Result

- Defined signed provider-tagged tickets, stop proofs, and no-writer receipts
  with typed verification failures. The start selector refuses while lifecycle
  and rollback support are pending.

### Why / context

- A host move needs authenticated evidence for the exact provider conversation,
  account, checkout, and stopped history snapshot.

### Evidence

- The baseline CLI fixture returned parse-error instead of the new typed
  refusal. A synthetic SSHSIG probe accepted the expected signer and refused
  altered payloads, a wrong principal, and a wrong namespace.

### Links

- <https://github.com/sympoies/nils-cli/issues/3981>
