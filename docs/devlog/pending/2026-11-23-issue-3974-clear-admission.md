## 2026-11-23 - Guard conversation clear before terminal input
<!-- devlog-id: 2026-11-23-issue-3974-clear-admission -->

### Result

- Clear checks provider attention, unread mailbox state, and the exact composer
  before terminal input or state mutation. The approved clear-only flag permits
  unread mail while preserving it.

### Why / context

- Clear sends a native slash command, so an idle projection alone could let it
  append to a human draft or interact with a provider dialog. The notice
  composer guard already has a read-only admission path.

### Evidence

- Synthetic Codex and Claude Code fixtures reproduced the missing guards before
  the fix. All 27 focused nextest cases passed after the fix, including clear,
  rebind, and conditional mailbox deletion.

### Links

- <https://github.com/sympoies/nils-cli/issues/3974>
