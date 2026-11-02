## 2026-11-03 - Use the configured session executable in mailbox reminders
<!-- devlog-id: 2026-11-03-agent-session-reminder-command-identity -->

### Result

- Mailbox reminder and recovery command text uses the existing executable-name
  owner; schema identities and default reminder bytes remain fixed.
- Read-back recognizes configured reminders and immutable default-name history,
  while the hook validates the configured command without executable aliases.

### Why / context

- Part A left reminder owners unchanged while their independent wake-format
  changes landed; this part completes the remaining command-name inventory.

### Evidence

- Configured reminder, history and quota assertions fail before production
  changes; the downstream hook independently rejects the renamed reminder.

### Links

- Refs #3193; follows #3236.
