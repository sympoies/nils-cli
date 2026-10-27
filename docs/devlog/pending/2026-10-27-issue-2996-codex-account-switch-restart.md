## 2026-10-27 - Recover queued Codex account switches on restart
<!-- devlog-id: 2026-10-27-issue-2996-codex-account-switch-restart -->

### Result

- Promote an explicitly queued Codex account over a failed binding during
  restart after configuring the replacement runtime, with a revision newer
  than both persisted records.

### Why / context

- A failed current account binding otherwise blocks input and control rebind
  even when a replacement account is queued.

### Evidence

- The preparation regression first failed because recovery checked account
  support while the replacement runtime still had its tmux kind. The fixed
  preparation tests cover app-server configuration before recovery and preserve
  the failed binding and queue when the broker no longer lists the account.
- All 50 Codex account tests and affected-package clippy passed after the fix.
  The earlier local-fast run, before the final restart changes, failed twice on
  an unrelated startup redaction test and stopped before nine tests ran. A later
  isolated run on that earlier source failed once and passed on retry; this did
  not make the local-fast run pass. Required CI remains the integration gate.

### Links

- <https://github.com/sympoies/nils-cli/issues/2996>
