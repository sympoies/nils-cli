# Development Guide

Principles and routine workflow for maintaining the `nils-cli` workspace.
Detailed setup, command inventories, validation lanes, coverage, generated
artifacts, and release/publish procedures live in the
[`workspace maintenance reference`](docs/runbooks/workspace-maintenance-reference.md).

## Maintenance principles

- Keep reusable behavior in the owning shared crate and preserve documented
  crate boundaries; do not duplicate cross-workspace policy in individual
  binaries.
- Treat agents and system automation as first-class CLI consumers. Preserve
  versioned schemas, stable error codes, deterministic exit status, bounded
  redacted diagnostics, and typed recovery metadata on new or touched paths.
- Keep work authorization distinct from collision awareness in
  `agent-session`: advisory coordination does not deny work, while strict
  admission belongs only to an explicit enforce launch.
- Keep source, completions, generated third-party artifacts, release metadata,
  and public documentation synchronized with their canonical owners.
- Every user-facing CLI exposes root `-V, --version`; clap parsers use
  `#[command(version)]` and show the flag in `--help`.
- In Rust tests, prefer `pretty_assertions::{assert_eq, assert_ne}` for useful
  diffs.
- Use changed-scope validation for the local loop. Full workspace, coverage,
  supply-chain, release, and live acceptance gates remain task-specific or
  CI-owned.

## Current contracts in this repository

This rule is owned by `nils-cli`; it is not a generic policy for consumers or
external projects. Maintain only the current repository contract. When changing
a format, contract, flag, path, name or schema, migrate all supported producers,
consumers, documentation and existing mutable records in the same change (or
an explicitly coordinated one-shot migration). Delete old readers, aliases,
fallbacks and dual writes. Retained signed records and evidence provenance stay
immutable; any necessary conversion creates a separate owner receipt.

External provider, OS and upstream-tool support is distinct: name the supported
external version and its contract when keeping that code. Local restore or replay
alone does not justify retaining an old repository contract.

Only cross-host wire/state interoperability between two named adjacent releases
may receive a bounded rollback window. The release owner must supply the exact
pair, cross-host justification, concrete first-disallowed release and hard calendar
expiry before merge. The release audit enforces that fence; neither a rolling
"next release" token nor indefinite retention is acceptable.

Existing debt is ratcheted by `scripts/ci/own-compat-audit.sh` and its exact
path/match/count TOML baseline. Each scheduled-removal row names its actual public
deletion issue, reason and hard expiry; reduce the count as occurrences disappear
and delete the row with the last occurrence. Matches use the emitted substring
with whitespace collapsed, without line-number coupling.

Rows describe existing debt and never authorize new old-form paths. The approved
2.0.0 train schedules CLI alias retirement in [N4](https://github.com/sympoies/nils-cli/issues/3076)
and [N5](https://github.com/sympoies/nils-cli/issues/3077), alongside the
[N6](https://github.com/sympoies/nils-cli/issues/3078) and
[N8](https://github.com/sympoies/nils-cli/issues/3080) breaking slices. Those aliases
still exist until their separately coordinated slices land; this policy/audit
change does not remove product behavior or authorize a breaking minor release.

The audit scans active source, scripts/wrappers, completions, tests and current
root/spec/runbook docs. Immutable devlogs/plans/discussions, build output and
third-party text are outside the active contract. It detects identifier forms,
serde/clap aliases, hidden JSON selectors, explicit retention declarations and
multi-version reader arms. Generic words, one current version constant,
prohibition text and explicitly rejecting test functions are not retention.

The audit, its synthetic fixture owner, baseline and existing tombstone scanner
are exact rule-definition files, not runtime retention examples. Other tests and
fixtures remain scanned. Exact expiring classifications distinguish current
contracts/provenance and versioned external support from scheduled own debt.
`current-contract` rows explain exact non-debt matches; `external` rows name the
supported external version and a review expiry within six months. Neither kind
grants permission to add own-version retention. The audit requires Python 3.11
or newer for the standard-library TOML parser.
Run `bash scripts/ci/own-compat-audit.sh --release X.Y.Z` before preparing a
release; normal checks also enforce fences against the workspace version.

## Development environment

Source the repository environment in every contributor or automation shell:

```bash
source scripts/dev-env.sh
```

This selects `DEVLOG_LAYOUT=fragments` for devlog writers. Install released
`devlog >=1.31.16`, or build `cargo build --locked -p nils-devlog` and add
`target/debug` to the shell's `PATH`. See the
[development log](docs/devlog/README.md) for entry authoring and fold ownership.

## Change workflow

1. Classify the change by crate, shared workspace surface, completion asset,
   generated artifact, documentation owner, coordination contract, or release
   boundary.
2. Inspect affected callers, tests, schemas, manifests, scripts, generated
   outputs, and canonical docs before editing.
3. Capture a meaningful regression failure for testable behavior when
   practical. Documentation-only work validates ownership, links, lint, and
   routing instead.
4. Make the smallest observable change and keep every affected package,
   completion, generated artifact, and contract test aligned.
5. Run focused checks while iterating, then the default local finish-line:

   ```bash
   bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast
   ```

6. Use the docs-only lane only when every changed path is documentation:

   ```bash
   bash scripts/ci/nils-cli-checks-entrypoint.sh --docs-only
   ```

7. Run full CI parity or coverage locally only for release-quality evidence,
   coverage maintenance, CI debugging, or an explicit request. GitHub required
   checks `test`, `test_containment`, and `cargo-deny (advisories + bans)`
   remain the Linux merge gate. Every merge group also requires affected-package
   `macos_clippy` (all targets/features, no test execution). Full Linux/macOS
   tests and coverage gate releases.
   See the [delivery and release flow](docs/runbooks/delivery-and-releases.md).

For an approved batch of low-risk changes, follow the
[`next` integration procedure](docs/runbooks/workspace-maintenance-reference.md#integration-batches-on-next).
Each member needs changed-scope validation PASS and review PASS on its exact head; the
batch shares one complete local gate on the frozen integration head.

## Documentation routing

| Need | Canonical document |
| --- | --- |
| Setup, builds, validation details, CI inventory, coverage, and publishing | [`Workspace maintenance reference`](docs/runbooks/workspace-maintenance-reference.md) |
| Runtime dependencies and degradation | [`BINARY_DEPENDENCIES.md`](BINARY_DEPENDENCIES.md) |
| CLI completion and alias policy | [`CLI completion standard`](docs/runbooks/cli-completion-development-standard.md) |
| New or redesigned CLI crates | [`New CLI crate standard`](docs/runbooks/new-cli-crate-development-standard.md) |
| Documentation ownership and placement | [`Crate docs placement policy`](docs/specs/crate-docs-placement-policy.md) |
| `agent-session` coordination | [`agent-session` docs](crates/agent-session/docs/README.md) |
| Temporary-directory cleanup | [`Test temp-directory policy`](docs/specs/test-temp-directory-policy.md) |
| Historical development narrative | [`Development log`](docs/devlog/README.md) |

`AGENTS.md` owns agent-specific repository rules. Plans, discussions, reports,
and the development log are retained evidence; they do not override current
source, schemas, policy, or canonical runbooks.
