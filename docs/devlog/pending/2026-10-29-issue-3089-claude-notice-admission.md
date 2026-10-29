## 2026-10-29 - Refuse unsupported Claude terminal notices before transport
<!-- devlog-id: 2026-10-29-issue-3089-claude-notice-admission -->

### Result

- Otherwise admitted Claude terminal notices now return
  notice-transport-unsupported before temporary input, buffer loading, paste,
  submission or a new delivery claim; the notice spec supersedes the narrower
  trailing-space refusal.

### Why / context

- Every notice envelope is multiline, and an opaque collapsed paste does not
  establish complete draft bytes or ownership for safe clearing. Full Claude
  notice delivery remains blocked under the parent issue.

### Evidence

- A modeled guarded-send regression failed before the production change with
  send-composer-mismatch at submit and pasted=true instead of the expected
  admission refusal. The migrated coverage checks receipt fields, no transport
  effects, fresh-key reuse once, queue termination and existing guard/Codex
  controls.

### Links

- <https://github.com/sympoies/nils-cli/issues/3089>
- <https://github.com/sympoies/nils-cli/issues/3068>

### Follow-ups

- Public-release pre-paste-refusal acceptance remains with the release owner;
  full native Claude delivery and owned-clear proof remain unresolved.
