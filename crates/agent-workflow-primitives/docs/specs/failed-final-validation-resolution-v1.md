# Failed final validation resolution v1

Final validation identity is command plus scope; attempt numbers increase within
that identity. A later PASS for one identity does not silently clear a FAIL for
another. When a canonical passing run covers the failed target on the same
source, record the investigation and disposition explicitly:

```sh
test-first-evidence show --out "$EVIDENCE_DIR" --format json
test-first-evidence resolve-final --out "$EVIDENCE_DIR" \
  --command 'cargo test affected' --scope affected-suite --attempt 1 \
  --passing-command 'bash scripts/check.sh' --passing-scope affected-suite \
  --passing-attempt 1 \
  --reason 'The canonical run covers the failed target on unchanged source' \
  --investigation 'Isolated base and head runs passed; investigation identified a transient fixture failure'
```

The maintainer must establish scope coverage, source equivalence and the failure's
cause before recording the disposition. The recorder validates references; it
cannot determine whether an investigation or source-equivalence claim is true.
A disposition is not a waiver for an unresolved product defect.

`resolve-final` identifies an existing failed attempt and adds an immutable
`resolution` object containing `passing_command`, `passing_scope`,
`passing_attempt`, `reason`, and non-empty `investigation` entries. The original
command, scope, attempt, FAIL status, summary and artifacts remain unchanged.
Old v2 records without `resolution` keep their existing behavior. Successful JSON
uses `cli.test-first-evidence.resolve-final.v1` and returns the full record.

Missing attempts, references to a failing or superseded passing attempt, blank
reason/investigation and replacement of an existing disposition are refused
without changing the file. While a failed identity remains current, its
resolution only satisfies readiness while the referenced validation identity
currently passes. A later PASS for that identity keeps the original disposition
visible; a later FAIL blocks delivery until a new passing run is recorded. Other unresolved failures, required validation
scopes, and repository/delivery subject checks continue to apply. If further
validation is needed, append a new final attempt; do not rewrite history.

When the evidence gate applies, `forge-cli pr create` appends a
`Final validation dispositions` section to the supplied body if the verified
record contains dispositions. It lists final attempts and their summaries,
including the original failures, referenced passes, resolution reasons and
investigation.

Standalone home-absolute references in validation commands,
referenced passing commands and investigation entries are displayed as
`$HOME`-relative references without changing stored command identities,
the record, resolution history or record hash.
Home-like components inside URIs remain unchanged so audit links cannot be
silently corrupted; the normal publication guard retains its conservative refusal.
Existing local-path and attribution checks still apply to all appended content.
Artifact paths and repository-local evidence locations are not copied.
The incomplete-evidence refusal names the exact `resolve-final` command shape.
