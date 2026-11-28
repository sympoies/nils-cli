## 2026-11-28 - Verify published Claude credentials before quarantine recovery
<!-- devlog-id: 2026-11-28-issue-4198-claude-quarantine-recovery -->

### Result

- Require a successful authenticated provider read for the exact publication
  and project typed per-session outcomes in board and glance.
- Share the conditional-retirement idle guard with recovery. Resume verified
  idle sessions and wait for the next idle point on busy sessions.
- Give Claude failover a bounded wait for replica credentials, then resume once
  or report a typed deadline without stopping the session.

### Why / context

- Credential publication and a ready replacement process do not by themselves
  prove provider acceptance.

### Evidence

- A synthetic publication replaced the runtime before the fix. The focused
  fake-provider test now keeps the quarantined runtime intact and reports a
  redacted, typed refusal.
- Synthetic replica tests reproduce immediate give-up before the fix and pass
  for delayed publication and deadline expiry with the bounded wait.
- Focused recovery and idle-retirement tests passed, including the same-account
  idle and busy siblings, the unrelated account, and both replica outcomes.

### Links

- <https://github.com/sympoies/nils-cli/issues/4198>
