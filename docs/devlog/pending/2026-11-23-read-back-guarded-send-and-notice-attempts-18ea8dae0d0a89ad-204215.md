## 2026-11-23 - Read back guarded send and notice attempts
<!-- devlog-id: 2026-11-23-read-back-guarded-send-and-notice-attempts-18ea8dae0d0a89ad-204215 -->

### Result

- A keyed attempt reports bound proof of submission, non-submission with
  composer state, or an unknown outcome.

### Why / context

- A timed-out transport can still submit, so recovery consumes the retained
  proof before sending one guarded Enter.

### Evidence

- Synthetic guarded-send fixtures and isolated Unix-socket WebSocket transport
  cover delayed proof, refusal, retained input, and replacement.

### Links

- <https://github.com/sympoies/nils-cli/issues/3673>
