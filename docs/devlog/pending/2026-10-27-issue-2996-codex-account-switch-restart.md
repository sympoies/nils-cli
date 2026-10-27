## 2026-10-27 - Recover queued Codex account switches on restart
<!-- devlog-id: 2026-10-27-issue-2996-codex-account-switch-restart -->

### Result

- Promote an explicitly queued Codex account over a failed binding during
  restart, with a revision newer than both persisted records.

### Why / context

- A failed current account binding otherwise blocks input and control rebind
  even when a replacement account is queued.

### Evidence

- The restart regressions cover promotion of a lower-revision queued account and
  preservation of state when the broker no longer lists that account. The
  existing direct queued-repair regression passed. Local-fast reported one
  unrelated flaky startup redaction test that passed on retry and in isolation.

### Links

- <https://github.com/sympoies/nils-cli/issues/2996>
