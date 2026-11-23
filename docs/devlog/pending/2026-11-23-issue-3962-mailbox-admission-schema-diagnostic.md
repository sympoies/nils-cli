## 2026-11-23 - Diagnose obsolete mailbox admission schemas
<!-- devlog-id: 2026-11-23-issue-3962-mailbox-admission-schema-diagnostic -->

### Result

- The daemon logs a fixed warning for an unsupported mailbox admission schema
  and points operators to the v2 migration steps.

### Why / context

- The strict v2 reader could reject a v1 document before checking its declared
  schema. The generic client refusal keeps admission state private.

### Evidence

- A synthetic loopback fixture verified that v1 still refuses submission,
  leaves the mailbox empty, and emits the warning locally. Both service mailbox
  cases and the federation fixture passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/3962>
