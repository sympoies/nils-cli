# Affected-scope merge plan v1

This document specifies the selection and trusted-input contract of the planner.
The plan it publishes is the [v2 contract](affected-scope-merge-plan-v2.md).

## Purpose and boundary

The standalone planner produces an affected-package decision for exact commits.
It does not execute checks, authorize merge, or replace a gate receipt. Current
local-fast behavior, required CI check names, and gate ownership remain unchanged.
CI execution of a plan is specified in
[affected-scope CI consumption](affected-scope-ci-consumption-v1.md).

Run the trusted launcher from an installation or checkout controlled by the
maintainer:

```bash
python3 scripts/ci/nils-cli-merge-plan.py \
  --repo "$REPOSITORY" --base "$BASE_COMMIT" --head "$REVIEWED_HEAD" \
  --candidate "$CANDIDATE_MERGE" --tree "$CANDIDATE_TREE"
```

`--repo` defaults to the current directory. B, H, and M must be full lowercase
commit object IDs; branch names and abbreviated IDs are refused. M must have
exactly two ordered parents, B then H. T is read from M; `--tree` optionally
asserts the expected T. The caller owns the authenticity of the trusted base,
the reviewed head, and candidate provenance. The planner does not construct M.
Git replacement objects and external diff/text conversion are disabled.

## Trusted inputs

The launcher reads and executes `scripts/ci/lib/merge_plan.py` from **B**, not
from H, M, or the working directory. That selector loads these files from B:

- [Policy](../../scripts/ci/merge-plan-policy.v1.json): versioned escalation,
  audit-only paths, completion asset templates and recognized documentation
  patterns.
- [Extra-input registry](../../scripts/ci/merge-plan-extra-inputs.v1.json):
  versioned non-Cargo inputs and their workspace consumers.
- [Output schema](../../scripts/ci/merge-plan.schema.v2.json): the v2 plan contract.

There is no candidate or working-directory fallback. A base that predates this
contract yields `PLAN_ERROR/HOLD`; initial rollout must land through the existing
complete gate. Policy, registry, selector, and schema edits under `scripts/`
therefore use the old trusted policy and escalate to FULL until they land.

Exact B and M trees are materialized in temporary directories from Git blobs,
including executable modes and safe relative symlinks. Case-folded or Unicode
normalization aliases (including directory prefixes) are rejected before writes
on every platform. Directory traversal and exclusive regular-file creation use
no-follow file descriptors; external symlinks or symlink path components HOLD.
`.gitattributes` export exclusions do not remove files from these snapshots. Submodules cannot be materialized by this
planner and yield HOLD. Neither the working tree nor index participates.

Both snapshots run metadata from the trusted B snapshot directory, with an
explicit manifest path for each snapshot:

```bash
cargo metadata --manifest-path "$SNAPSHOT/Cargo.toml" \
  --offline --locked --all-features --format-version 1
```

No target filter or `--no-deps` is used. Cargo and rustc executable paths are
bound from the caller's trusted PATH, compiler wrappers are disabled, and rustup
auto-install is disabled. Candidate configuration and toolchain files never
select metadata tooling. Cargo, rustc, the trusted base toolchain and registry
cache must already be available. The caller must provide its normal
resource containment and environment for metadata; this command does not
compile packages, create a test gate, or manage a gate slot.

By default it does not fetch either. When
`NILS_CLI_MERGE_PLAN_PREPARE_METADATA=1`, each snapshot first runs the same
command without `--offline`, so Cargo may download locked dependencies that a
restored cache lacks; build scripts do not run. That output is discarded:
selection still consumes the offline, locked run above. The CI `test` and
`test_macos` jobs set it after restoring the Cargo cache.

Root Cargo configuration in either snapshot, or any changed `.cargo`/toolchain
input, yields `METADATA_FAILED` at `candidate_configuration` before Cargo runs.
This conservative HOLD also applies to unchanged root configuration: includes,
program selection and relative dependency overrides cannot be evaluated from B
while claiming an exact M graph. Internal root toolchain symlink bytes are
compared as well.

Caller environment/cache and toolchain availability can affect whether planning
succeeds. Any metadata failure, invalid resolved graph, or workspace root outside the snapshot yields
HOLD, including documentation-only and always-escalate diffs.

## Selection rules

Changes are the exact B→M `git diff --name-status -z --find-renames=50% -l0`
records. NUL framing preserves whitespace and non-UTF-8 filenames (escaped in
JSON). Rename records include both old and new paths. Deletions, executable-mode
changes, and symlink type changes participate. Unknown or malformed diff records
fail closed.

Workspace packages are mapped by manifest directory. Package IDs, rather than
renamed dependency imports or package names, key the union of the B and M
resolved graphs. Snapshot-root portions of path IDs normalize to `repo:`; output
manifest paths are repository relative. Reverse-dependency traversal reaches a
fixed point through all resolved packages, including external intermediaries.
Normal, optional, target-specific, dev, and build edges are included. Only
candidate workspace packages are selected for validation. A deleted or
unmappable owner or consumer escalates to FULL. A nonempty selection covering
every candidate workspace member is FULL with `ALL_WORKSPACE_PACKAGES`, even
when it arose only from reverse-dependency closure.

Selection order for each changed path is:

1. Cargo custom-build target paths from either tree escalate to FULL.
2. A path listed in the policy's `audit_only` section (the file-size, docs-prose
   and test-stale baselines) records `AUDIT_ONLY`, selects no package and does
   not escalate: the scoped lane runs the audit that reads each baseline over
   the whole workspace. A diff of only such paths selects nothing, so the
   consumer runs FULL (`empty_selection`).
3. A path matching a `completion_assets` template, such as
   `completions/zsh/_{bin}` or `completions/bash/{bin}`, selects the one
   candidate workspace package that has a binary target named `{bin}` and
   records `COMPLETION_OWNER`. An unknown or ambiguous binary escalates as
   `UNKNOWN_COMPLETION_OWNER`; a deleted owner has no binary in the candidate.
   Any other `completions/**` path (adapters, alias files) falls through.
4. Always-escalate patterns take precedence over documentation and extra-input
   rules. The policy includes manifests/lockfiles, toolchain/build inputs,
   shared crates and test support, CI/configuration, every other `scripts/**`
   and `tests/**` path, remaining completions, sensitive lifecycle/credential
   surfaces, and outward schemas in workspace or crate `docs/specs/*.schema.json`
   contract locations. Test fixture schemas remain owned by their crate. Crates
   with no workspace dependents, such as `nils-forge-cli` and
   `nils-agent-session`, are ordinary owned crates.
5. Map the path to its workspace owner in either tree.
6. Apply **every** matching extra-input rule, including rules for paths already
   owned by a crate. Add each consumer and its transitive reverse dependencies.
7. Recognized documentation with no owner or registry rule records a documentation
   reason. Every other unknown path escalates to FULL.

The registry uses case-sensitive `fnmatch` patterns over repository-relative
POSIX paths (`*` can include `/`). Each rule has a unique `id`, nonempty `paths`,
and nonempty `packages` containing Cargo workspace package names. Missing or
removed consumers escalate to FULL; malformed registry or policy versions HOLD.
In-crate fixtures and generated/include files map to their owning crate by
default; register additional consumers when Cargo does not represent the edge.
The initial registry covers binary consumers, a shared identity fixture, and an
included workspace runbook. A rule selects a package to test, with its whole
reverse closure, so it names only a package whose tests run the changed binary
(`semantic-commit` selects `nils-forge-cli`, not `nils-common`). The registry does not claim to discover arbitrary
runtime dependencies automatically.

## Output and failure contract

Stdout is one canonical JSON object matching `nils-cli.merge-plan.v2`. Objects
have sorted keys; package, reason, and change arrays are sorted and deduplicated
where appropriate. There are no timestamps, command diagnostics, or temporary
paths. Identical inputs and successful metadata graphs produce identical bytes.

- `status: READY`, `action: VALIDATE`, exit 0: planning succeeded. `scope` is
  `SCOPED`, `FULL`, or `NONE`; `selected_packages` identifies candidate workspace
  packages by ID/name/manifest. FULL selects every candidate workspace package.
  Documentation can have an empty package selection; NONE means an empty diff.
  Neither case is a validation PASS or merge authorization.
- `status: PLAN_ERROR`, `action: HOLD`, exit 1: no usable scope. `scope` is null
  and package/reason arrays are empty. The fixed `error.code` and `error.stage`
  identify input, selector, policy, diff, snapshot, Git, or metadata failure.
  Retry after repairing inputs; stderr from Git/Cargo is never published in the
  plan. Argument-parser usage errors exit 2 before planning.

`inputs` binds B/H/M/T. `digests` binds SHA-256 of the trusted selector, policy,
registry, schema, raw NUL diff, and normalized base/candidate package graphs.
Graph digests cover workspace ID/name/manifest mappings, all resolved reverse
edges (including external nodes), and custom build sources. Available digests
are retained on failure. `digests.plan` is SHA-256 of canonical JSON of the full
object **before** inserting that digest itself. No digest is a signature or proof
of successful validation; consumers must check status/action and exact inputs.

## Validation

The selector and command matrix uses Python 3.9 or later and `jsonschema` for schema
validation, plus Git and Cargo for disposable fixtures:

```bash
python3 scripts/ci/tests/merge-plan.test.py
```

The matrix covers both-tree/transitive/dependency-kind selection, external
intermediaries, renamed imports, old/new rename names, deletion, executable and
symlink changes, extra inputs, escalation precedence, unknown inputs, complete
metadata failures, input binding, trusted-base isolation, schema validity, and
byte-for-byte deterministic output. Safe-link and proper-subset controls
accompany alias/no-follow, compiler sentinel, whole-workspace and public-schema
regressions. A real Cargo workspace with only path
dependencies exercises optional, target-specific, dev, and build dependencies
without network access or compilation. The suite is separate from existing CI
required-check and gate entrypoints.
