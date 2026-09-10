# Test-First Evidence Mutation Receipt v1

## Scope

This contract governs successful JSON output from:

- `test-first-evidence record-failing`
- `test-first-evidence record-final`
- `test-first-evidence bind-delivery`

The persisted `test-first-evidence.record.v2` file remains the durable source
of truth and is not changed by this response contract.

## Output modes

With `--format json`, the three commands return a compact mutation receipt by
default. Their envelope schemas are respectively:

- `cli.test-first-evidence.record-failing.v3`
- `cli.test-first-evidence.record-final.v3`
- `cli.test-first-evidence.bind-delivery.v3`

Passing `--full-record` returns the complete accumulated record using the
corresponding pre-existing v2 command envelope. Text mode is unchanged;
`--full-record` only changes JSON detail.

## Compact receipt

Every v3 result contains:

- `record_file`: the written evidence file.
- `complete`: whether the record is structurally complete after this mutation.
- `mutation.effect`: `appended` for these append-only commands.
- `mutation.item`: exactly the failing evidence, final validation, or delivery
  attempt written by this invocation.
- `subject`, when the record is bound: repository and immutable baseline
  identity plus the latest delivery as `delivery`, if one exists.

The compact result never contains `record`, `failing_tests`,
`final_validations`, or `deliveries` history arrays.

```json
{
  "schema_version": "cli.test-first-evidence.bind-delivery.v3",
  "command": "test-first-evidence bind-delivery",
  "ok": true,
  "result": {
    "record_file": "/tmp/evidence/test-first-evidence.json",
    "complete": true,
    "mutation": {
      "effect": "appended",
      "item": {
        "head": "<git-object-id>",
        "tree": "<git-object-id>",
        "diff_digest": "sha256:<digest>",
        "attempt": 2
      }
    },
    "subject": {
      "repository": {
        "kind": "provider",
        "id": "github.com/acme/widget"
      },
      "baseline": {
        "commit": "<git-object-id>",
        "tree": "<git-object-id>"
      },
      "delivery": {
        "head": "<git-object-id>",
        "tree": "<git-object-id>",
        "diff_digest": "sha256:<digest>",
        "attempt": 2
      }
    }
  }
}
```

The machine-readable schema is
[`test-first-evidence-mutation-receipt-v1.schema.json`](test-first-evidence-mutation-receipt-v1.schema.json).
