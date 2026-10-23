## 2026-10-24 - Keep mailbox wakes observable in narrow composers
<!-- devlog-id: 2026-10-24-issue-2875-narrow-mailbox-wake -->

### Result

- Use a fixed 35-character mailbox wake for Claude and Codex; preserve exact
  observation of older dated attempts with versioned receipt formats.

### Why / context

- Dated wakes wrap at 51 columns and cannot pass the existing exact composer
  guard. Shortening the wake avoids relaxing draft protection or
  unread-generation fences.

### Evidence

- Test-first narrow composer and detached Claude dispatcher failures; 115
  focused mailbox, composer, notification, and remote-inbox tests passed.
  Formatting, strict file-size and docs-hygiene checks passed.

### Links

- <https://github.com/sympoies/nils-cli/issues/2875>

### Follow-ups

- The delivery steward owns full-suite validation and real detached-provider
  acceptance before merge.
