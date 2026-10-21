# Workspace documentation

Workspace-level runbooks and specs for `nils-cli`. Crate-specific documents
live under `crates/<crate>/docs/` and are indexed by each crate's
`docs/README.md`; the [retention matrix](specs/workspace-doc-retention-matrix-v1.md)
records ownership for both.

## Start here

- [README](../README.md): workspace overview, CLI surface map, and install.
- [DEVELOPMENT.md](../DEVELOPMENT.md): maintenance principles and the routine
  workflow.
- [Workspace maintenance reference](runbooks/workspace-maintenance-reference.md):
  setup, validation lanes, coverage, generated artifacts, and publishing.
- [Delivery and release flow](runbooks/delivery-and-releases.md): CI lanes,
  merge gates, and release ownership.
- [Development log](devlog/README.md): conventions and the month index.

## Building a CLI crate

- [New CLI crate development standard](runbooks/new-cli-crate-development-standard.md)
- [Crate and CLI naming convention v1](specs/crate-cli-naming-convention-v1.md)
- [CLI help style guide](runbooks/cli-help-style-guide.md)
- [CLI output contract v1](specs/cli-output-contract-v1.md): output envelope
  and exit codes.
- [CLI service JSON contract guideline v1](specs/cli-service-json-contract-guideline-v1.md)
- [Workspace shared crate boundary v1](specs/workspace-shared-crate-boundary-v1.md)
- [Markdown template development standard](runbooks/markdown-template-development-standard.md)

## Completions

- [CLI completion development standard](runbooks/cli-completion-development-standard.md)
- [Completion contract template](specs/completion-contract-template.md)
- [Completion coverage matrix v1](specs/completion-coverage-matrix-v1.md)

## CI, tests, and review

- [Workspace CI entrypoint inventory v1](specs/workspace-ci-entrypoint-inventory-v1.md)
- [Affected-scope merge plan v1](specs/affected-scope-merge-plan-v1.md),
  [v2](specs/affected-scope-merge-plan-v2.md), and
  [CI consumption v1](specs/affected-scope-ci-consumption-v1.md)
- [Test temporary-directory policy](specs/test-temp-directory-policy.md)
- [Test cleanup governance](runbooks/test-cleanup-governance.md) and the
  [test cleanup lane matrix v1](specs/workspace-test-cleanup-lane-matrix-v1.md)
- [Review specialists primitive](runbooks/review-specialists-primitive.md)

## Release and artifacts

- [crates.io status script runbook](runbooks/crates-io-status-script-runbook.md)
- [Third-party artifacts contract v1](specs/third-party-artifacts-contract-v1.md)

## Documentation policy

- [Crate docs placement policy](specs/crate-docs-placement-policy.md)
- [Workspace doc retention matrix v1](specs/workspace-doc-retention-matrix-v1.md)
