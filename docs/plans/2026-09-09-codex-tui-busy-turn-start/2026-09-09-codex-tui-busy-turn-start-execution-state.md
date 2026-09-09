# Codex TUI Busy Turn Start — Execution State

<!-- plan-issue-record:v2 role=state profile=tracking -->

## Execution State

- Status: active
- Source document: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-plan.md`
- Implementation source: `docs/plans/2026-09-09-codex-tui-busy-turn-start/2026-09-09-codex-tui-busy-turn-start-discussion-source.md`
- Direct source-doc execution waiver: not applicable.
- Tracking issue: <https://github.com/sympoies/nils-cli/issues/1644>
- Branch: `fix/codex-tui-busy-turn-start`
- Worktree: managed by `git-cli worktree`
- Active task: 2.1
- Last checkpoint: pre-merge review findings recorded on PR #1646
- Current task: 2.1
- Next task: validate, review, and merge

## Task Ledger

| ID | Status | Task | Evidence | Notes |
| --- | --- | --- | --- | --- |
| 1.1 | done | Capture live-shape regressions | Controlled init and no-init freebox canaries reproduce local `-32001`; Retained owner test failed before the fix; live init and no-init freebox canaries reproduced account_not_ready | Failure occurs before provider acceptance. |
| 1.2 | done | Repair authorization boundary | Focused turn-start, account-authority, and manual-input tests pass; isolated no-init and post-init canaries returned exact expected replies | Proxy trusts only durable bound plus exact runtime; daemon retains broker mutation authority. |
| 1.3 | done | Add privacy-safe rejection evidence | JSON-RPC error preserves code/message and adds closed-vocabulary data.reason; runbook and API spec updated | No user-controlled or identifying fields are emitted. |
| 2.1 | pending | Validate, review, and merge | none | Independent testing and maintainability review required. |
| 2.2 | pending | Release, deploy, and live verify | none | Pane-preserving restart only. |
| 2.3 | pending | Close historical and current tracking | none | Repair #1631 state without changing its scope. |

## Scope decisions

- This is a new defect line linked to, but not a reopening of, delivered issue
  #1631 and merged PR #1632.
- Account switching is not the root cause; the local proxy rejects before the
  provider sees the turn.
- Automatic replay remains forbidden because a generic transport failure can
  have an ambiguous delivery outcome.

## Handoff

Continue at Task 2.1 in the managed worktree. Repair the finite pre-merge
finding set, then retain privacy-safe release and live acceptance evidence.
