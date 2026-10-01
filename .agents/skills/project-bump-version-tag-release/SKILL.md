---
name: project-bump-version-tag-release
description: Version-bump preparation for nils-cli releases; tagging, publishing, and fleet convergence belong to the sympoies-infra release broker.
---

# Nils CLI Release Preparation

Releases are driven by the private release broker, not by hand. Do not tag,
push tags, publish a GitHub Release, dispatch the Homebrew tap, or upgrade
hosts from this repository. To release, use the owner skill
`project-release-nils-cli` in `serenvia/sympoies-infra`
(`.agents/skills/project-release-nils-cli`); the release decision belongs to the
maintainer or laoda.

## What stays in nils-cli

The version bump itself. The entrypoint
`.agents/skills/project-bump-version-tag-release/scripts/project-bump-version-tag-release.sh`
(reached through `.agents/scripts/release.sh`) is the canonical bump
transform. The broker consumes it through
`.github/workflows/prepare-private-release.yml`, which runs it on a hosted
runner and uploads only a patch and a checksum-bound manifest. The broker then opens and lands the release PR as the
single commit `chore(release): bump cli versions to X.Y.Z`.

The transform:

- bumps the workspace and crate `Cargo.toml` versions and the internal
  `path` dependency pins;
- updates the README release tag example;
- refreshes `Cargo.lock` with `cargo update --workspace` and validates with
  `cargo check --workspace --locked`;
- regenerates `THIRD_PARTY_LICENSES.md` and `THIRD_PARTY_NOTICES.md`.

Unless `--prepare-only` is given it then validates, stages, and makes the
single local commit `chore(release): bump cli versions to X.Y.Z`.
`--prepare-only` applies the same transform and exits before validation,
staging, and commit; it is the contract mode used by tests. The script has no
tag, push, tap, or install path and rejects any such flag.

A canonical version-only release PR may use the reduced release-only CI lane
only when protected base policy recognizes it and the exact base `main` SHA has
a trusted successful full CI run; otherwise it falls back to full PR CI.

## What belongs to the broker

Release PR delivery and merge, tagging, the source `release.yml` run, the
Homebrew tap update, and fixed-fleet convergence are owned by the broker and its
runbook (`modules/release-nils-cli/docs/release-operations.md` in
`serenvia/sympoies-infra`). Do not recreate them here.

## When changing the bump transform

Edit the script and run its test:

```bash
bash .agents/skills/project-bump-version-tag-release/tests/test_bump_version_tag_release.sh
bash scripts/ci/tests/release-workflow-contract.test.sh
bash scripts/ci/tests/detect-release-only.test.sh
```

The script is preparation-only; the release path is the broker.
