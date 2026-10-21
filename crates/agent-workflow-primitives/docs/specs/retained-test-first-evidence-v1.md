# Retained test-first evidence v1

`test-first-evidence retro-bind` accepts an initialized, unbound
`test-first-evidence.record.v2` record and writes the dedicated
`test-first-evidence.record.v3` format. Older v2 writers reject that format
before mutation, while current writers preserve its provenance. Use this
recovery only when the original meaningful RED preceded production edits and
GREEN followed the implementation, but record initialization or baseline
binding was missed. Normal work still uses `init` and `bind-baseline` before
editing.

## Recovery

```bash
test-first-evidence init --out "$EVIDENCE_DIR" \
  --classification bug-fix --changed-behavior 'The regression is fixed'
test-first-evidence retro-bind --out "$EVIDENCE_DIR" --project-path . \
  --baseline "$ORIGINAL_BASE_COMMIT" \
  --red-log "$RED_LOG" --green-log "$GREEN_LOG" \
  --red-command 'cargo test regression' --green-command 'cargo test regression' \
  --red-exit-code 101 --green-exit-code 0 \
  --expected-failure 'New contract assertion fails' \
  --observed-failure 'Expected value differs from actual value' \
  --reason 'Original logs were retained before initializing the record' \
  --attest-pre-edit-red --format json
test-first-evidence record-impact --out "$EVIDENCE_DIR" \
  --target tests/regression.rs --disposition add-missing \
  --protected-behavior 'The regression stays fixed' --reason 'Missing coverage'
test-first-evidence record-gap --out "$EVIDENCE_DIR" --none
# Commit the implementation, complete any other required validation scopes,
# and attest the actual delivery head.
test-first-evidence bind-delivery --out "$EVIDENCE_DIR" --project-path .
test-first-evidence verify --out "$EVIDENCE_DIR" --project-path . --format json
```

Use the original full baseline commit ID, which must exist and be an ancestor
of the current head. Symbolic refs such as `HEAD` are rejected. Both logs must
be readable and nonblank. The caller supplies the actual commands, RED's
nonzero exit code, GREEN's zero exit code, meaningful expected/observed
failure descriptions, a reason, and an explicit chronology attestation.
Setup, compilation, or unrelated failures do not qualify as meaningful RED.

The command refuses existing subjects, RED/GREEN entries, and waivers; it
never replaces prior evidence or baseline bindings. Rejected input leaves the
record unchanged. A v1 record remains read-only. No commands are rerun and no
baseline is inferred from the implementation head.

## Record and response

Success uses `cli.test-first-evidence.retro-bind.v2` with `command`, `ok: true`,
and the usual full `result` containing `record_file`, `complete`, and `record`.
The v3 record receives one meaningful failing entry and one focused passing
validation. Existing impact and gap declarations are preserved. Delivery
attestations remain empty until `bind-delivery` runs.

The required `subject.retro_binding` object makes recovery explicit:

| Field | Meaning |
| --- | --- |
| `mode` | Always `retro-bound` |
| `reason` | Redacted nonblank explanation for late binding |
| `pre_edit_red_attested` | Explicit caller chronology declaration, always true |
| `bound_at` | Timestamp of the import, never the original run time |
| `bound_head` | Full commit ID at import time |
| `red_log_digest`, `green_log_digest` | SHA-256 of the original retained bytes, prefixed `sha256:` |

Artifact paths are stored using the existing redaction rules. Log contents
are never copied into the record or response. Digests identify imported bytes;
they cannot establish when a test ran or prove that supplied exit codes and
descriptions are truthful. Reviewers inspect the original logs and chronology.
Logs need not remain readable for later verification; the digests retain their
audit identity. Keep the original logs available to the reviewer privately.

Missing or malformed v3 provenance makes the record incomplete and prevents
current writers from mutating the record. Ordinary v2 records omit this object
and retain their existing behavior. Existing repository, baseline,
delivery-head, and diff checks still apply to imported records.

## Verification guidance

`verify --project-path` reports structural omissions and missing baseline or
delivery subjects together. `incomplete-evidence` retains exit code 1 and its
`details.missing` array, adding `details.next_steps` entries with `field` and
`command`. Text output lists the same missing fields and recovery commands.
Subject-only failures retain exit code 65 and the existing `unbound-subject`
or `subject-mismatch` error codes and `reason_code`, with recovery guidance.
Without `--project-path`, verification remains structural only.

Imported GREEN satisfies only the focused scope. Required affected-suite,
contract-consumer, full, or manual validation and unresolved failed attempts
must still be handled through the existing record and resolution commands.
