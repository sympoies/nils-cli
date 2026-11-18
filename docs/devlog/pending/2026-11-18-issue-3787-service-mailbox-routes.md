## 2026-11-18 - Identify missing service mailbox federation routes
<!-- devlog-id: 2026-11-18-issue-3787-service-mailbox-routes -->

### Result

- Service-send distinguishes absent daemon federation, rejected relay service
  discovery and refused service admission, retaining fixed typed recovery
  through the daemon and CLI.

### Why / context

- Service and managed-session sends already share the loopback daemon, journal
  and transport. A hub accepting session discovery can reject service
  selectors; local service admission cannot supply hub support or authority.
  Diagnose that boundary without borrowing session credentials or inventing
  relay configuration.

### Evidence

- Synthetic two-machine loopback service mailbox acceptance: baseline failed on
  generic remote-messaging-unavailable; both focused fixture tests pass with
  daemon/CLI error equality, persisted managed remote delivery and local
  service submission.

### Links

- <https://github.com/sympoies/nils-cli/issues/3787>
