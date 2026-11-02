# Affected-scope merge plan v2

V2 adds typed metadata failure recovery to the
[v1 selection and trusted-input contract](affected-scope-merge-plan-v1.md).
Ordered B/H/M/T binding, trusted-base selector loading, offline/locked
all-feature metadata, both-tree dependency closure and escalation are unchanged.
The [v2 schema](../../scripts/ci/merge-plan.schema.v2.json) is the only plan
schema. The CI consumer accepts only v2 plans and holds any other version with
the same input and digest checks.

## Failure diagnostics

Metadata errors retain METADATA_FAILED, their base/candidate stage,
PLAN_ERROR/HOLD, null scope and empty selection. Optional error.recovery
describes the failure without publishing exception paths or metadata stdout:

| kind | Additional fields | Meaning |
| --- | --- | --- |
| missing_tool | tool: cargo or rustc | Caller executable unavailable |
| spawn_error | Optional numeric errno | Tool lookup or invocation failed |
| cargo_exit | Numeric exit_code, stderr_tail | Cargo metadata exited unsuccessfully |
| json_invalid | None | Metadata stdout was not valid JSON |
| workspace_root_mismatch | None | Root was missing, malformed or outside the snapshot |
| graph_error | None | Resolved graph failed normalization |

stderr_tail is at most 2,048 UTF-8 bytes. It retains only allowlisted Cargo
explanations rendered using producer-owned wording. Unknown lines are replaced
with a fixed redaction marker, including unlabeled credentials, registry aliases
and unquoted paths containing spaces. It copies no runtime-derived prose,
package names, URLs, identifiers or paths. Truncation follows rendering.
Original process status remains separately typed; automation branches on fields
instead of parsing the tail. Non-metadata errors can retain empty recovery.

The canonical plan digest includes recovery details. HOLD remains unsuccessful
planning even when the consumer safely passes actual FULL fallback. A redacted
or unrecognized explanation does not establish a cache/toolchain root cause.

## Activation and validation

The launcher still loads its selector and schema from B. A new diagnostic in H
cannot change a predecessor B's output; detailed live failures become available
when the new selector is in a later run's accepted base. No schema fallback or
candidate-selected planner is introduced.

The metadata fixture covers each failure class, bounded private-content
handling, canonical serialization, v2-only consumer acceptance and other-version
HOLD using mocked tools and fake Cargo. The original selector matrix
retains its assertions and validates v2 output. CI provisions Python 3.12 and
its schema fixture dependency for both platforms, then runs the fixture in the
normal full audit stack.

These tests establish contracts. A successful local metadata-only reproduction
does not identify a failed hosted caller's lost diagnostic, prove a live scoped
run, or measure throughput. Those results remain separate acceptance work.
