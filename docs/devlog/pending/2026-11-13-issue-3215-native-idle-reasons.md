## 2026-11-13 - Diagnose native guarded idle refusals without provider values
<!-- devlog-id: 2026-11-13-issue-3215-native-idle-reasons -->

### Result

- Native guarded send and notice refusals identify runtime, transport, RPC or
  status observation failures with fixed reason labels and recovery hints;
  durable prompt receipts and notice deferrals retain the typed reason.

### Why / context

- The original idle-session report could not be diagnosed from one shared
  refusal code. Naming the failed observation makes a later occurrence
  actionable without weakening idle, identity or idempotency checks or
  retaining provider payloads.

### Evidence

- The new missing-status reason assertion failed on main before production
  edits. Native send/notice fixture cases and runtime, persistence and privacy
  controls pass after the change.

### Links

- <https://github.com/sympoies/nils-cli/issues/3215>
