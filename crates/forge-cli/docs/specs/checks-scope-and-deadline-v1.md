# Check scope and wait deadlines

This additive contract supplements [forge-cli v1](forge-cli-spec-v1.md) for
`pr checks` and `pr wait-checks`. Both retain `cli.forge-cli.pr.checks.v1`.

## Scope fields

Both JSON payloads expose these fields alongside the existing check rows.
Default text output prints the same fields after its existing summary line:

| Field | Meaning |
| --- | --- |
| `configured_required_count` | Number of normalized rows marked `required`; gate snapshots include pending rows for configured requirements that have not registered. |
| `gate_scope` | `required` for required rows, `visible` when all visible rows are explicitly selected, or `visible_fallback` when GitHub has no required rows. |
| `gate_count` | Number of rows in the effective gate, including visible fallback rows. |
| `gate_state` | Aggregate state for that effective gate. |
| `visible_count` | Number of reported rows, including pending placeholders in gate snapshots. |
| `visible_state` | Aggregate state across all reported rows. |
| `required_skipped` | Number of effective-gate rows (the `gate_count` set) whose state is `skipped`. |
| `required_neutral` | Number of effective-gate rows whose state is `neutral`. |
| `skipped_required` | The `skipped` and `neutral` effective-gate rows as `{name, state, url?, conclusion?}`; empty when every gate row ran. |

With no required checks and one successful visible check, both commands report
`configured_required_count=0`, `gate_scope=visible_fallback`, `gate_count=1`,
and `visible_state=success`. The row remains `required=false`.

The existing `required_count` is deprecated for scope interpretation. It and
`success_count`, `state`, `failed`, and `pending` retain their existing meaning:
the one-shot command aggregates the requested subset; the waiter applies the
visible fallback when no GitHub check is required. Consequently, the one-shot
command can retain `required_count=0` while the waiter reports `required_count=1`.
New consumers use `gate_count` and `gate_state` for the effective gate and
`visible_state` for the complete reported set. One-shot reads retain their
rollup boundary; waiters use complete configuration and exact-head reads.

## Skipped and neutral gate rows

By default, `gate_state=success` keeps GitHub ruleset semantics: a required check that
concluded `skipped` or `neutral` satisfies the rule, and `state`,
`success_count`, `failed`, and `pending` are unchanged. It is not proof the
work ran: a check deferred to a later gate, such as a merge group, concludes
`skipped`. Consumers that must prove execution read `required_skipped`,
`required_neutral`, and `skipped_required` (`success_count` below `gate_count`
means some gate rows did not succeed); they do not infer it from `gate_state`.

When `skipped_required` is non-empty, default text output adds, after the scope
line, one `required_skipped=N required_neutral=N (...)` line and one
`skipped-required: <name> (<state>)` line per row, indented two spaces. Text
output is unchanged when every gate row succeeded or failed. Rows outside the
effective gate are never listed; outside `gate_scope=required` the effective
gate is the visible rows, so those rows are reported under the same names.

## Strict one-shot execution gate

`pr checks <id> --strict` opts into an execution gate. Every row in the
effective gating set must conclude `success`; `skipped` and `neutral` count
as failures. The default remains the provider-compatible snapshot above.
This flag applies only to `pr checks`; wait, merge, and delivery retain their
existing policies.

In strict mode, skipped and neutral gating rows set both `state` and
`gate_state` to `failure` and join `failed` with their original name, URL,
and conclusion (the normalized state is used when no conclusion is supplied).
The original `checks` rows, skip counts, `skipped_required`, `success_count`,
and `visible_state` retain their reporting semantics. Text output shows the
failure summary and the skipped/neutral check names. No JSON fields are removed.

A non-passing strict gate emits `ok=false` with the snapshot in `data` and exits
`RUNTIME 1`: `error.code=checks_failed` for failed, cancelled, timed-out, skipped,
or neutral gating checks, or `checks_pending` when only pending checks block.
A successful strict gate emits the ordinary success snapshot and exits zero.
An empty gating set retains the ordinary empty-snapshot behavior; strict mode
does not establish required-check configuration or prove snapshot completeness.

With `--required-only=true`, optional skipped or neutral rows do not block a
required gate. With `--required-only=false`, or with GitHub's
`visible_fallback` scope when no checks are required, every visible row gates
and is subject to strict mode. For example, a required `build=success` and
`test=skipped` return `gate_state=success`, exit zero by default, and
`gate_state=failure`, exit 1 with `--strict`.

## Wait budget

`pr wait-checks --timeout` shares one monotonic deadline across identity and
Git repository/configuration discovery, provider detection, and snapshot calls,
identity preparation inside those calls, rate-limit probes, waits, retries,
and the optional workflow probe. Backend subprocesses receive only the budget
remaining when they start. Poll sleeps are capped by the same deadline.
A terminal result arriving at or after expiry cannot return success.

Expiry returns the last fully observed snapshot and elapsed `duration_ms`.
During polling, every expiry path, including an interrupted backend read,
returns `checks_not_registered` (exit 65) if no check was ever observed in a
complete snapshot. It returns `checks_timeout` (exit 69) after checks were
observed but did not finish, even if later snapshots are empty. If no snapshot
completed, the payload has `state=pending` and no rows; partial or late data
cannot replace a previously observed snapshot. The classification depends on
observed history rather than which deadline check or poll sleep ends the wait.
Discovery timeout before any snapshot uses the ordinary error envelope with
`checks_timeout` (exit 69); no provider snapshot is available yet. Explicit no-checks opt-outs and
positive no-workflow detection retain their existing behavior within the budget.

Process startup, scheduling, and subprocess cleanup can add a small tolerance
to wall time. The deadline does not restart for a second snapshot or backend.
