## 2026-11-02 - Find a lost-acknowledgement native prompt by its client message id
<!-- devlog-id: 2026-11-02-issue-3051-native-client-message-id -->

### Result

- A keyed idle-only Codex prompt now sends a provider-visible client message id
  derived from the dispatch-time session identity (runtime, provider thread,
  account binding) and the request id. When the turn acknowledgment is lost and
  that identity is unchanged, lookup or a repeated POST finds that one user
  message by its id and the exact claimed text, and changes the receipt to
  submitted without another dispatch. A changed identity computes a different id
  and matches nothing.

### Why / context

- Refs 3051 and 3186: a lost acknowledgment left no way to tell a delivered
  prompt from an undelivered one, so the receipt stayed unknown. The pinned
  Codex protocol retains the id on the user message, and an installed 0.162.1
  build returned it on thread read and across a restart.

### Evidence

- RED before the change: the new test failed because turn/start carried no
  client message id; the other six native reconciliation tests passed.
- GREEN after the change: all seven native reconciliation tests pass, including
  the case that leaves a nearby matching row without an id unknown.

### Follow-ups

- The busy-reply route that uses the control handle sends no id yet. A missing
  row stays unknown. This gives causality only, not a different message role,
  so the display requirement of 3051 is unchanged.
