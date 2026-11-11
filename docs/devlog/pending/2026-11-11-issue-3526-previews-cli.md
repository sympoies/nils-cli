## 2026-11-11 - agent-session: local read of last_prompt and History previews
<!-- devlog-id: 2026-11-11-issue-3526-previews-cli -->

### Result

- Added `agent-session previews <id>`, a read-only local command returning one
  session's `last_prompt` projection and History START/LATEST rows.

### Why / context

- Acceptance of History and preview changes needed these values without Console
  access or the serve credential.

### Evidence

- TODO
