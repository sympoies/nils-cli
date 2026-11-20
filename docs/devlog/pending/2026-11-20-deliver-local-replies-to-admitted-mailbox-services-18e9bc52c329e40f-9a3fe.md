## 2026-11-20 - Deliver local replies to admitted mailbox services
<!-- devlog-id: 2026-11-20-deliver-local-replies-to-admitted-mailbox-services-18e9bc52c329e40f-9a3fe -->

### Result

- Added exact-generation local service replies and credential-scoped inbox,
  show and acknowledgement.

### Why / context

- The local daemon binds replies to the authenticated parent origin and keeps
  retired service generations isolated.

### Evidence

- The synthetic fixture reproduced mailbox-service-reply-unsupported before the
  change. The service and federation fixtures passed after local delivery was
  added.

### Links

- <https://github.com/sympoies/nils-cli/issues/3797>

### Follow-ups

- Convert the private admission document alongside the implementing daemon in
  the same deployment.
