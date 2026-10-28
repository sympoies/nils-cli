# claude-cli Docs

This crate owns Claude-specific CLI helpers that have moved out of zsh-kit shell scripts.

- Crate README: `../README.md`
- [JSON contract v1](specs/claude-cli-json-contract-v1.md): stable usage,
  prompt-segment status, redacted auth status, and `diag rate-limits`
  envelopes.
- [Auth reset-rate-limits JSON contract v1](specs/claude-cli-auth-reset-rate-limits-json-contract-v1.md):
  the Claude limit-reset redemption command.
- [Usage consumer runbook](runbooks/usage-consumer.md): safe parsing and
  `agent-session` integration rules.
- Workspace standards: `docs/runbooks/new-cli-crate-development-standard.md`
  and `docs/runbooks/cli-completion-development-standard.md`.
- [Credential writer receipts v1](specs/claude-cli-writer-receipt-v1.md)
