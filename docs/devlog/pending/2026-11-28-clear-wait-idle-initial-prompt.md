## 2026-11-28 - Fence conversation clear and its initial prompt
<!-- devlog-id: 2026-11-28-clear-wait-idle-initial-prompt -->

### Result

- Conversation clear can wait for idle, submit one initial prompt on the
  verified new identity, and retain a digest-only receipt.

### Why / context

- Native confirmation requires a record-lock handoff. A lifecycle input fence
  keeps managed input and mailbox wakes out until clear and the first prompt
  have finished.

### Evidence

- Fake Claude and Codex fixtures exercise working-to-idle admission, draft
  refusal, exactly-once prompt submission, wake deferral, and confirmation
  timeout without resend.

### Links

- <https://github.com/sympoies/nils-cli/issues/4200>
