# Agent Session Runtime Recovery — Execution State

<!-- plan-issue-record:v2 role=state profile=tracking -->

## Execution State

- Status: active
- Source document: `docs/plans/2026-09-08-agent-session-runtime-recovery/agent-session-runtime-recovery-plan.md`
- Implementation source: `docs/plans/2026-09-08-agent-session-runtime-recovery/agent-session-runtime-recovery-discussion-source.md`
- Direct source-doc execution waiver: not applicable.
- Tracking issue: <https://github.com/sympoies/nils-cli/issues/1631>
- Branch: `fix/agent-session-runtime-recovery`
- Worktree: managed by `git-cli worktree`
- Active task: 2.1
- Last checkpoint: Pre-merge review findings repaired and follow-up review passed

## Task Ledger

| ID | Status | Task | Evidence | Notes |
| --- | --- | --- | --- | --- |
| 1.1 | done | Empty-tmux regression | Retained test-first red | Missing server returned unavailable before the fix. |
| 1.2 | done | Empty-state classification | Focused unit and reconnect-fence tests pass | Unknown failures remain unavailable. |
| 1.3 | done | Busy-input recovery docs | API contract and runbook updated | `prompt/v2` remains exact-incarnation fenced. |
| 2.1 | in-progress | Validate, review, merge | Local-fast, provider checks, and focused review pass | Merge pending. |
| 2.2 | pending | Release, deploy, live verify | none | Blocked by provider merge. |

## Scope decisions

- The startup defect is owned by `nils-agent-session`, not Agent Console UI.
- Busy-input recovery remains explicit because raw terminal submission can have an ambiguous outcome.
- The local busy response alone is not evidence of non-delivery; recovery first
  requires provider-visible confirmation that no turn accepted the continuation
  or remains in progress.
- Successful malformed tmux output retains its pre-existing parser semantics;
  the new fail-closed guarantee is limited to unrecognized non-success results.
- Provider-visible evidence excludes private hostnames, local paths, session identifiers, prompt content, and credentials.
