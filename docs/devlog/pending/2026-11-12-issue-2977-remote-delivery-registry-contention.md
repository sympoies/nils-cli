## 2026-11-12 - Keep queued remote mailbox sends answerable under registry contention
<!-- devlog-id: 2026-11-12-issue-2977-remote-delivery-registry-contention -->

### Result

- Remote delivery lookup and same-key message send replay authenticate from the
  committed registry snapshot and answer from the federation journal, so a
  queued remote message no longer waits on the coordination registry lock; a
  refused snapshot reruns the locking authentication. A remaining registry lock
  timeout on message send or delivery now carries retryable, next_action and
  recovery details.

### Why / context

- The send was durably queued before the failing step; only the follow-up
  delivery check and same-key retry took the whole-registry lock, so registry
  contention returned a bare coordination-lock-timeout and the caller could not
  tell queued from failed. The lock wait stays bounded at two seconds.

### Evidence

- The two-root remote mailbox acceptance fixture queues a message behind a held
  relay, holds the source registry lock, and failed before the fix (delivery
  lookup timed out) and passes after it, including idempotency-key conflict,
  single destination copy and recovery details on the remaining timeouts.

### Links

- <https://github.com/sympoies/nils-cli/issues/2977>
