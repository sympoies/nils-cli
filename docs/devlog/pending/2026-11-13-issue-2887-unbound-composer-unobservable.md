## 2026-11-13 - Bound unobservable-composer wakes for unbound Codex records
<!-- devlog-id: 2026-11-13-issue-2887-unbound-composer-unobservable -->

### Result

- A terminal-started Codex wake with no provider binding and an unobservable
  composer is recorded as `recipient-composer-unobservable` and settles
  `undeliverable` on the fifth consecutive refusal of the generation, instead
  of retrying every second until the message expires. The first four refusals
  keep the one-second retry.
- `recipient-working`, `provider-not-ready`, the multi-row Claude composer
  retry, records with a provider binding and Console-started Codex records are
  unchanged.

### Why / context

- The attempt path was already bounded (#3345, #3618); the pre-attempt refusal
  path was bounded only by message expiry (#2887).

### Evidence

- The new tests cover the unbound settlement and the bound-record retry; the
  unbound test failed before the change. The mailbox spec documents the
  terminal state and the bound.
