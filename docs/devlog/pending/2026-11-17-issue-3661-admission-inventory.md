## 2026-11-17 - Isolate admission capacity from host process inventory
<!-- devlog-id: 2026-11-17-issue-3661-admission-inventory -->

### Result

- Kept stopped-session admission coverage through the CLI and moved
  deletion-capacity assertions into the existing injected-inventory deletion
  regression.

### Why / context

- The inventory is diagnostic context, not proof of the teardown failure cause.
  A successful retry need not have a complete inventory. Admission capacity
  should not depend on unrelated procfs entries or real cgroup teardown
  scheduling.

### Evidence

- Before editing, a denied procfs read for an owned unrelated probe process
  deterministically failed the original delete assertion with
  verification-failed and no blocking processes. The retained record
  distinguishes this reproduction from the unproven historical trigger.

### Links

- <https://github.com/sympoies/nils-cli/issues/3661>
