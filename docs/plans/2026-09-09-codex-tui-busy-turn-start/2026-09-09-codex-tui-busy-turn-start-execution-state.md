# Codex TUI Busy Turn Start — Execution State

<!-- plan-issue-record:v2 role=state profile=tracking -->

## Execution State

- Status: complete
- Source document: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-plan.md`
- Implementation source: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-discussion-source.md`
- Direct source-doc execution waiver: not applicable.
- Tracking issue: <https://github.com/sympoies/nils-cli/issues/1644>
- Branch: `docs/codex-tui-busy-closeout`
- Worktree: managed by `git-cli worktree`
- Active task: none
- Last checkpoint: v1.28.13 fixed-fleet convergence and production canaries passed
- Current task: complete
- Next task: none
- Branch/commit/PR: sympoies/nils-cli#1646 merged (<https://github.com/sympoies/nils-cli/pull/1646>)
- Last updated: 2026-09-09

## Task Ledger

| ID | Status | Task | Evidence | Notes |
| --- | --- | --- | --- | --- |
| 1.1 | done | Capture live-shape regressions | Controlled init and no-init freebox canaries reproduce local `-32001`; Retained owner test failed before the fix; live init and no-init freebox canaries reproduced account_not_ready | Failure occurs before provider acceptance. |
| 1.2 | done | Repair authorization boundary | Focused turn-start, account-authority, and manual-input tests pass; isolated no-init and post-init canaries returned exact expected replies | Proxy trusts only durable bound plus exact runtime; daemon retains broker mutation authority. |
| 1.3 | done | Add privacy-safe rejection evidence | JSON-RPC error preserves code/message and adds closed-vocabulary data.reason; runbook and API spec updated | No user-controlled or identifying fields are emitted. |
| 2.1 | done | Validate, review, and merge | Local-fast passed 1,290 tests; all hosted checks and independent review passed; PR #1646 merged at `3def58d292cb7efba024b5f474ecdfc2580dd24f` | All seven review findings were resolved before merge. |
| 2.2 | done | Release, deploy, and live verify | v1.28.13 release and fixed-fleet broker run succeeded; serve restarted from 1.28.13 with the same six-session inventory; full Agent Console smoke passed; freebox no-init and init-then-next canaries returned the exact expected replies | Restart retained `KillMode=process`; disposable canaries were removed. |
| 2.3 | done | Close historical and current tracking | Historical #1631 remained closed with its execution state repaired in PR #1646; current #1644 passed strict closeout and is closed | No historical scope was reopened. |

## Scope decisions

- This is a new defect line linked to, but not a reopening of, delivered issue
  #1631 and merged PR #1632.
- Account switching is not the root cause; the local proxy rejects before the
  provider sees the turn.
- Automatic replay remains forbidden because a generic transport failure can
  have an ambiguous delivery outcome.

## Handoff

- Tracking issue <https://github.com/sympoies/nils-cli/issues/1644> is closed.
  This final commit synchronizes that terminal state in the repository; no
  runtime or issue action remains.
