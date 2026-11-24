## 2026-11-24 - Bound unobserved mailbox notification attempts
<!-- devlog-id: 2026-11-24-notification-attempt-fence -->

### Result

- Serve releases a notification submission fence after 120 seconds without
  exact observation. Serve and lifecycle admission release the fence for
  stopped, replaced, or auth-quarantined runtimes.

### Why / context

- A crashed or unobservable submission could indefinitely prevent restart of
  its target. An unknown outcome consumes the original attempt and preserves
  the delivery evidence.

### Evidence

- Pre-fix nextest reproduced expired and stopped receipts remaining in
  attempting while exact positive observation still passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/4007>
