## 2026-10-28 - Share Claude usage fetches across local readers
<!-- devlog-id: 2026-10-28-claude-shared-usage-readers -->

### Result

- OAuth readers reuse one account-scoped response for 60 seconds and one
  cooldown; diagnostic tables explain throttling and highlight an unrecorded
  active login.

### Why / context

- A cooldown alone serialized failures but still allowed every successful
  reader to poll independently. Sharing the status-query superset under the
  same lock avoids duplicate reads while preserving snapshot age; cross-host
  authority remains a separate transport contract.

### Evidence

- Claude CLI unit and integration tests pass, including concurrent mixed
  processes, account aliases, server retry hints, shared stale snapshots, and
  active-login fallback.

### Links

- <https://github.com/sympoies/nils-cli/issues/3039>
