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

## Wait budget

`pr wait-checks --timeout` shares one remaining budget across snapshot calls,
identity preparation inside those calls, rate-limit probes, waits, retries,
and the optional workflow probe. Backend subprocesses receive only the budget
remaining when they start. Poll sleeps are capped by the same deadline.
A terminal result arriving at or after expiry cannot return success.

Expiry returns the last fully observed snapshot and elapsed `duration_ms`.
When checks were observed or no snapshot completed, an interrupted backend
read returns `checks_timeout` (exit 69). If no snapshot completed,
the payload has `state=pending` and no rows; partial or late data cannot replace
a previously observed snapshot. A completed series of empty snapshots retains
`checks_not_registered` (exit 65) on expiry. Explicit no-checks opt-outs and
positive no-workflow detection retain their existing behavior within the budget.

Process startup, scheduling, and subprocess cleanup can add a small tolerance
to wall time. The deadline does not restart for a second snapshot or backend.
