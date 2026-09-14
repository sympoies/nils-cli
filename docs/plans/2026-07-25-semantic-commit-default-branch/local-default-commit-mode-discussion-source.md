# Local-Default Commit Mode Implementation Handoff

- **Status**: decided; implementation-ready; no unresolved design questions
- **Date**: 2026-07-23
- **Source**: In-session requirements and design discussion about governed
  commits directly on a primary checkout's local default branch
- **Intended next step**: implement the coupled `nils-cli` and
  `agent-runtime-kit` changes locally, validate them against source-built
  binaries, and leave signed commits on each repository's local `main` without
  invoking GitHub issues, pull requests, or Actions
- **Implementation state after local completion**: source-complete and locally
  validated; provider delivery, release, tap publication, and version-pin
  promotion remain explicitly deferred until provider access is usable

## Purpose

Add a narrow L0 local-default completion mode for maintainers who explicitly
want one signed commit authored directly on the primary checkout's local
default branch. The mode removes managed-worktree ceremony for a bounded local
hotfix or documentation change while preserving checkout isolation, exact-base
binding, semantic commit validation, signature verification, structured
evidence, and a safe later path to provider delivery.

The mode works whether or not Git remotes are configured. When a remote exists,
the caller must explicitly choose a local-only remote outcome. The command
never contacts or mutates a remote. It records that the provider has not
received the commit and does not weaken any later deploy, release, or
direct-main provider gate.

## Confirmed Facts

- Current runtime-kit policy requires agent-authored commits to be created on a
  non-default managed-worktree branch. `semantic-commit` mutations on the
  checked-out default branch are blocked by the default-delivery hook. [F1]
- The primary checkout already has a narrow direct-edit exception when it is
  clean, on the resolved default branch, outside a Git operation, and free of a
  live foreign checkout lease. That exception currently authorizes editing
  isolation only, not a commit or delivery mode. [F1]
- `semantic-commit commit` already owns semantic message validation, staged-file
  commit creation, `--require-clean`, `--expect-head`, `--repo`, dry-run, and a
  structured JSON result. A local-default authoring mode belongs in this binary
  rather than a second commit implementation in `git-cli`. [F2]
- `forge-cli repo push-default` already owns the provider-bound direct-main
  compare-and-swap path. It validates an exact remote base, one signed commit,
  fast-forward ancestry, a bounded reason file, a unique provider-bound push
  URL, and post-push remote read-back. It currently rejects a checked-out
  default branch. [F3]
- The default-delivery hook intentionally blocks only unsafe default-branch
  authoring and raw provider pushes; hooks are mechanical guardrails rather than
  a complete shell sandbox. [F4]
- Durable shared CLI behavior belongs in `sympoies/nils-cli`; runtime-kit owns
  the natural-language policy, hook routing, rendered product surfaces, version
  consumption, and acceptance coverage. [F5]
- GitHub is currently unusable for this work because the account or repository
  has been classified as spammy; issues, pull requests, and Actions must not be
  used during the local implementation run. This is a user-provided operating
  constraint, not a claim verified against provider APIs. [U1]

## Decisions

1. Add a dedicated `semantic-commit local-default` subcommand. Do not add a
   `git-cli repo` command and do not add a permissive flag to ordinary
   `semantic-commit commit`.
2. The command authors one commit directly on the branch checked out by the
   repository's primary worktree. It does not create, switch, promote, or
   remove a linked worktree.
3. The expected branch is caller-bound through required
   `--expected-branch <name>` input. Runtime-kit policy requires the exact
   branch named by the maintainer's current-task authorization, normally
   `main`; the CLI never guesses a no-remote repository's default branch.
4. Required `--expect-head <full-sha>` binds the commit to the exact local base.
   The command reuses the existing commit engine's precondition and additionally
   verifies after commit that the new commit's sole parent equals that SHA.
5. The command forces cryptographic commit signing and verifies the created
   commit locally. It exposes no `--no-gpg-sign`, amend, fixup, squash,
   message-only, or allow-empty path.
6. The command requires staged changes and rejects every unstaged or untracked
   path. `--require-clean` behavior is mandatory rather than caller-optional.
7. Zero configured remotes requires no remote option. One or more configured
   remotes requires exact `--remote-mode local-only`; omission or any other
   value fails closed.
8. Remote presence is not a network precondition. The command performs no
   fetch, `ls-remote`, provider lookup, push, or other network operation.
   Connectivity and provider availability therefore cannot block local commit
   creation.
9. When the checked-out branch has a locally resolvable upstream tracking ref,
   its pre-commit relation must be `aligned` or `ahead-only`. `Behind`,
   `diverged`, unresolved, or otherwise unknown ancestry fails closed.
   A branch with no configured upstream is allowed and recorded as `untracked`;
   a configured upstream whose cached ref cannot be resolved fails closed.
10. A successful remote-present result is explicitly local-only. If the cached
    upstream was aligned, the postcondition is `ahead-by-one`; if it was already
    ahead by N commits, the postcondition is ahead by N+1; otherwise it remains
    `untracked`. Canonical v1 serialization keeps `ahead-by-one` for a count of
    one and uses `ahead-by-<decimal>` without a leading zero for counts of two
    or more. No new receipt field is introduced. Neither state is presented as
    live provider truth.
11. The mutating command requires `--receipt-out <path>`. The CLI preflights a
    new, non-symlink destination outside the repository worktree, writes the
    receipt atomically, and refuses to overwrite an existing file. Runtime-kit
    workflows allocate this path through `agent-out`.
12. The local receipt is terminal evidence only when the maintainer explicitly
    requested local-default completion in the current task. The words “small”,
    “hotfix”, or “docs” never imply authorization.
13. Local-default completion is an L0-only exception: exactly one signed commit,
    no issue, no PR, no provider mutation, and no cross-session state ledger.
    A pre-existing ahead-only gap may come from earlier completed tasks, but it
    does not enlarge the current authorization. Work that cannot finish in the
    current run or requires multiple new commits is retained on a managed branch
    and re-triaged.
14. A later provider push is a new action with fresh authorization. Extend
    `forge-cli repo push-default` with
    `--local-default-receipt <path>` so it can adopt the exact governed local
    commit, re-run every live remote/signature/ancestry check, and push through
    the existing compare-and-swap path.
15. Receipt adoption is the only new exception to `push-default`'s current
    rejection of a checked-out default branch. Without a valid receipt, its
    non-default managed-worktree rule remains unchanged.
16. A local receipt never bypasses project-owned deploy or release gates. A
    repository that requires `local main == remote main` continues to fail
    closed until provider delivery or explicit repository policy says otherwise.
17. No new user-facing skill is added. `local-default` is an internal primitive
    selected by the implementation/delivery parent after authorization and
    preflight.
18. During the current GitHub spammy period, implementation uses no GitHub
    issue, PR, review, merge, workflow, release, or Actions mutation. Local
    source commits are the requested terminal state; provider promotion is a
    later, separately authorized task.

## User-Facing Command Contract

### No configured remotes

```bash
semantic-commit local-default \
  --message-file <path> \
  --expect-head <full-local-head-sha> \
  --expected-branch main \
  --receipt-out <agent-out-path> \
  --automation \
  --format json
```

### One or more configured remotes

```bash
semantic-commit local-default \
  --message-file <path> \
  --expect-head <full-local-head-sha> \
  --expected-branch main \
  --remote-mode local-only \
  --receipt-out <agent-out-path> \
  --automation \
  --format json
```

The command accepts the existing ordinary-commit message sources and structured
message fields (`--message`, `--message-file`, `--type`, `--scope`, `--subject`,
`--body-bullet`, `--signoff`, and `--trailer`). Automation uses a prepared
message file or structured fields and never falls back to stdin.

The command supports `--repo <path>`, `--dry-run`, `--validate-only`,
`--auto-fix`, `--summary`, `--message-out`, `--no-progress`, and `--quiet` when
their existing semantics remain read-only or do not weaken the contract.
`--expect-head`, `--expected-branch`, and `--receipt-out` are required for a
mutating invocation. `--remote-mode local-only` is conditionally required by
configured remote count.

The command rejects `--amend`, `--no-edit`, `--message-only`, `--allow-empty`,
and all fixup/squash semantics. It provides no force, reset, branch creation,
branch switch, push, retry, or automatic rollback option.

## Preconditions And State Resolution

Before creating a commit, the implementation must prove all of the following:

1. `--repo` or the current directory resolves to a non-bare Git repository.
2. The target is the primary worktree reported by `git worktree list
   --porcelain`, not a linked worktree.
3. `HEAD` is attached and its full branch name equals `--expected-branch`.
4. `HEAD^{commit}` equals the full object ID resolved from `--expect-head`.
5. No merge, rebase, cherry-pick, revert, bisect, or `git am` operation is in
   progress.
6. At least one staged path exists.
7. No unstaged or untracked path exists.
8. Effective signing configuration is usable. The commit is invoked with
   signing required; signing failure must leave `HEAD` unchanged.
9. The receipt parent exists, is outside the target worktree, is not a symlink,
   and can accept an atomic no-clobber write.
10. Configured remotes are counted using local Git configuration only. Remote
    URLs are not printed or stored in the receipt.
11. When remotes exist, `--remote-mode` is exactly `local-only`.
12. If `@{upstream}` exists, its cached commit resolves and is either equal to
    the current pre-commit `HEAD` or an ancestor of it. A cached upstream ahead
    of local `HEAD`, divergent history, unresolved ref, or unknown ancestry
    fails closed. If there is no upstream, state is `untracked`.

Checkout ownership remains a runtime-kit hook responsibility. Codex and Claude
must hold or acquire the ordinary checkout writer lease before edits, staging,
or the local-default invocation. The CLI still performs all repository-local
preconditions so direct human use fails safely without depending on an agent
hook.

## Mutation And Postconditions

The command must:

1. Capture the exact old `HEAD`, branch, staged tree, remote count, and cached
   upstream relation.
2. Create one signed semantic commit whose parent is the captured old `HEAD`.
3. Resolve the new full commit and tree object IDs.
4. Verify the new commit signature as locally good.
5. Verify that the branch still equals `--expected-branch`, `HEAD` equals the
   new commit, the parent equals the expected old commit, and the worktree plus
   index are clean.
6. Recompute the cached upstream relation without contacting the network.
7. Atomically create the structured receipt.
8. Emit the same result on stdout when JSON output is selected.

The command does not automatically roll back a commit after Git reports commit
success. If a post-commit invariant or receipt finalization fails, it returns a
typed partial-success error containing the new SHA and recovery guidance, keeps
the commit for inspection, and does not amend, reset, or delete it.

## Local Receipt Contract

The new schema is `cli.semantic-commit.local-default.v1`. The canonical payload
has this shape:

```json
{
  "schema_version": "cli.semantic-commit.local-default.v1",
  "ok": true,
  "data": {
    "mode": "local-default",
    "repository_fingerprint": "sha256:<digest>",
    "branch": "main",
    "old_head": "<full-sha>",
    "new_head": "<full-sha>",
    "parent_sha": "<full-sha>",
    "tree_sha": "<full-sha>",
    "signature": "verified-good",
    "staged_file_count": 1,
    "remote": {
      "configured_count": 1,
      "mode": "local-only",
      "network_observed": false,
      "provider_mutated": false,
      "upstream": "origin/main",
      "cached_relation_before": "aligned",
      "cached_relation_after": "ahead-by-one"
    },
    "completion": {
      "local_default_committed": true,
      "provider_delivered": false,
      "provider_reconciliation_required": true
    }
  }
}
```

`repository_fingerprint` is a privacy-safe digest bound to the canonical Git
common directory and object format; the receipt contains no absolute path,
remote URL, commit-message body, diff, filename, user identity, key material,
or provider credential. The upstream branch name is local Git metadata and may
be omitted when state is `untracked`.

Receipt files are local runtime evidence under `agent-out`, never tracked in a
working repository and never copied verbatim into provider bodies. A later
provider command may consume the file locally and expose only bounded delivery
facts. `completion.provider_delivered` is always `false`, including when
`cached_relation_before` was already ahead-only.

## Later Provider Delivery

Add this optional receipt-adoption form to the existing command:

```bash
forge-cli repo push-default \
  --local-default-receipt <path> \
  --expected-base <full-remote-sha> \
  --reason-file <path> \
  --format json
```

Receipt mode must revalidate rather than trust the file:

- receipt schema, bounds, regular-file/no-symlink shape, and repository
  fingerprint;
- current checkout is the primary worktree on the provider-resolved default
  branch;
- current `HEAD` equals receipt `new_head`;
- receipt `old_head`, `parent_sha`, and caller `--expected-base` are identical;
- the remote default branch still equals the expected base;
- exactly one commit exists in the base-to-head range;
- ancestry is fast-forward and the signature is locally good;
- checkout and index are clean;
- selected push URL, provider repository, rewrite protections, bounded reason
  file, exact-old-object lease, push, and post-push read-back satisfy the
  existing `push-default` contract.

An ahead-only local gap may contain commits from earlier local tasks. The newest
receipt does not make that multi-commit gap eligible for single-commit adoption:
receipt adoption remains restricted to the canonical `aligned` to
`ahead-by-one` relation pair, and the live `--expected-base` plus exact existing
one-commit range checks must also pass.

Fresh explicit maintainer authorization is required for this provider mutation.
The earlier local-default authorization cannot be reused. If the live remote
has moved, the command fails without retrying or rewriting local history; the
maintainer then chooses a PR/reconciliation task when provider access is
available.

## Runtime Policy And Hook Integration

Update runtime-kit so the delivery matrix has three distinct outcomes:

| Mode | Authorization | Terminal evidence | Provider state |
| --- | --- | --- | --- |
| PR | Ordinary implementation request | Provider PR/MR merge read-back | Delivered |
| Direct-main | Current-task direct commit-and-push authorization | `push-default` remote-SHA receipt | Delivered |
| Local-default | Current-task local-default/no-push authorization | `semantic-commit local-default` receipt | Not delivered |

Required policy changes:

- `AGENT_HOME.md`: add the narrow L0 local-default route without weakening the
  non-default authoring rule for every other mode.
- `core/policies/work-tier-levels.md`: keep PR default; define local-default as
  an L0-only local completion, not a provider delivery shortcut.
- `core/policies/git-delivery.md`: add the command contract, remote-present
  acknowledgement, receipt semantics, local cleanup proof, later provider
  adoption, and failure behavior.
- `core/policies/intent-cards.md`: route exact authorized local-default work
  through `project-dev`; do not infer it from size or urgency.
- `core/policies/files-hooks-validation.md`: keep receipts under `agent-out` and
  prohibit committing them.

Required hook changes:

- Teach shared semantic-commit parsing that `local-default` is a distinct
  mutating subcommand.
- Allow it on the checked-out expected branch only when the exact governed
  command shape is visible; continue blocking ordinary `commit`, `fixup`, and
  `squash` there.
- Keep checkout lease, project-dev intent, commit-body, secret scan, and
  finish-line validation gates active.
- Keep raw `git commit` and raw pushes to the provider default branch blocked.
- Block common raw local-default bypasses that advance or rewrite the checked
  default branch from a local commit (`git merge`, `cherry-pick`, destructive
  reset, direct `update-ref`, and equivalent aliases) while preserving exact
  recovery forms and provider-origin `pull --ff-only` cleanup.
- Recognize `forge-cli repo push-default --local-default-receipt` as the only
  governed checked-default provider route; ordinary checked-default
  `push-default` remains rejected.

Hooks enforce common agent paths but do not claim to sandbox arbitrary local
processes. The CLI rechecks every invariant and remains authoritative for
direct human invocation.

## Implementation Boundaries

### `sympoies/nils-cli`

- `crates/semantic-commit/src/cli.rs`: register `local-default`.
- `crates/semantic-commit/src/local_default.rs`: own parsing, preconditions,
  signed commit execution, postconditions, receipt creation, typed errors, and
  text/JSON rendering while reusing the ordinary commit message engine.
- `crates/semantic-commit/src/commit.rs`: expose the smallest internal message,
  staged-entry, expected-head, clean-state, and Git-runner helpers needed by the
  new module; do not duplicate semantic validation.
- `crates/semantic-commit/tests/integration/`: add command, receipt, remote
  state, signing, race, partial-success, output-safety, and completion tests.
- `crates/nils-common/`: add a shared strict receipt parser/type only if
  `forge-cli` consumption would otherwise duplicate the schema. Keep parsing
  bounded and reject unknown schema versions.
- `crates/forge-cli/src/cli.rs`: add `--local-default-receipt` to
  `repo push-default` with mutual-exclusion rules for ordinary authoring mode.
- `crates/forge-cli/src/ops/repo_push_default.rs`: add receipt adoption while
  preserving the existing provider binding, exact lease, and remote read-back.
- `crates/forge-cli/tests/integration/repo_push_default.rs`: cover valid
  adoption and every stale, forged, cross-repository, moved-remote, unsigned,
  dirty, wrong-branch, or multi-commit rejection.
- Shell completion and CLI help snapshots: include the new subcommand and flag.

### `agent-runtime-kit`

- Update the five policy surfaces named above and their rendered home-prompt
  mirrors.
- Update `core/hooks/shared/block-unsafe-default-delivery.py` and shared
  semantic-commit effect parsing.
- Add focused cases to `tests/hooks/test_shared_hooks.py` and the agent-hook
  contract suite.
- Update `docs/source/nils-cli-surface.md`, CLI inventories, required floors,
  runtime-smoke cases, rendered product output, and goldens only after a real
  nils-cli release exists.
- During provider outage, validate the runtime-kit implementation against the
  source-built nils-cli binary with `scripts/dev/with-nils-version.sh local`.
  Do not move `minimum_supported_tag`, `validated_tag`, Homebrew pins, or
  released CLI floors to an unreleased local commit.
- Do not add a skill, provider workflow, issue reference, PR reference, or
  Actions dependency for the local implementation.

## Provider-Outage Local Execution Contract

The current implementation run is deliberately local-only. It must not invoke
`forge-cli issue`, `forge-cli pr`, `gh`, `glab`, GitHub release commands,
provider review commands, or Actions APIs.

Implementation may use managed worktrees while the new command and hook support
are not yet active. Final local delivery occurs only after the source-built
`semantic-commit local-default` and updated hook path pass their focused tests:

1. Implement and validate the nils-cli changes on a managed non-default
   worktree using the existing signed `semantic-commit` path.
2. Implement and validate runtime-kit policy, hook, and acceptance changes on a
   separate managed worktree against the source-built nils-cli binaries.
3. Reapply each verified repository diff to its clean primary `main`, preserving
   the exact validated content and declared validation results.
4. Use the source-built `semantic-commit local-default` to create one signed
   commit on each local `main`, with `--remote-mode local-only` where remotes
   are configured and receipts allocated outside the repositories.
5. Re-run the focused post-commit validation against each local-main SHA and
   verify both receipts.
6. Remove the implementation worktrees through `git-cli worktree remove` only
   after local receipt read-back proves local `main` equals the retained commit.
   Delete disposable branches only after the same local proof.

If the updated hook cannot be activated safely for the self-hosting commit, the
implementation remains on its signed managed branches and reports the exact
activation blocker; it does not bypass the hook with raw Git. This is a typed
failure condition, not an unresolved design choice.

Expected local terminal state:

- `sympoies/nils-cli`: clean local `main` containing one signed implementation
  commit; any configured upstream is unchanged.
- `graysurf/agent-runtime-kit`: clean local `main` containing one signed
  integration commit; `origin/main` is unchanged.
- Local receipts: verified under `agent-out`, not committed.
- Provider artifacts: none.
- Release/pin status: pending provider recovery; not falsely marked released or
  validated.

## Scope

- Dedicated local-default semantic commit authoring.
- No-remote and remote-present local-only operation.
- Primary-worktree, branch, base, clean-state, operation-state, signing, and
  receipt enforcement.
- Cached upstream classification without network access.
- Local terminal evidence and cleanup rules.
- Optional later `push-default` receipt adoption with fresh authorization.
- Runtime-kit policy, hooks, render surfaces, tests, and release-consumption
  boundaries.
- Fully local implementation commits while GitHub mutations are unavailable.

## Non-Scope

- Multiple local-default commits in one task.
- Behind, diverged, unresolved, or unknown upstream reconciliation.
- Unsigned commits, force updates, reset-based delivery, auto-stash, automatic
  rollback, amend/fixup/squash, or history rewriting.
- Automatically pushing after local commit.
- Treating cached upstream state as live provider truth.
- Bypassing deploy/release remote-alignment checks.
- Provider issue, PR, review, merge, release, or Actions activity during the
  current spammy period.
- Moving released version floors or pins to an unreleased local binary.
- Installing or activating changed live runtime surfaces without the ordinary
  explicit maintainer approval and readiness checks.

## Requirements

- **R1**: `semantic-commit local-default` creates exactly one signed commit on
  the primary worktree's expected local branch from the expected old HEAD.
- **R2**: The command needs no managed worktree for its consumer hotfix/docs use
  case.
- **R3**: Configured remote count greater than zero requires
  `--remote-mode local-only`; remote connectivity is irrelevant and no network
  process runs.
- **R4**: Aligned or ahead-only cached upstream state is allowed; one current
  authorization creates exactly one new commit and records the prior count plus
  one. Behind, diverged, unresolved, or unknown state fails; untracked state is
  allowed and explicit.
- **R5**: No unstaged/untracked paths, empty commits, unsigned commits, Git
  operations, branch drift, expected-base drift, or linked-worktree targets are
  admitted.
- **R6**: Successful mutation produces a bounded, privacy-safe, atomic local
  receipt outside the repository.
- **R7**: Runtime-kit treats the receipt as terminal local evidence only for an
  explicitly authorized L0 local-default task and never as provider evidence.
- **R8**: Later provider push requires fresh authorization and full
  `push-default` revalidation through receipt adoption.
- **R9**: Raw commit/push and common local-default bypasses remain blocked by
  hooks; ordinary provider fast-forward cleanup remains available.
- **R10**: The provider-outage implementation performs no issue/PR/Actions or
  other GitHub mutation and ends in signed local-main commits plus local
  receipts.
- **R11**: Unreleased source validation does not move released floors, pins, or
  validated-version claims.
- **R12**: No new skill is introduced.

## Acceptance Criteria

### `semantic-commit`

- **A1**: Help, completion, dry-run, validate-only, text, and JSON forms expose
  the exact decided interface.
- **A2**: A primary `main` with no remotes, one staged change, expected HEAD,
  clean residual state, and usable signing creates one verified commit and a
  valid v1 receipt.
- **A3**: A repository with a configured remote rejects omission or drift of
  `--remote-mode local-only`; the exact mode succeeds without executing any
  network command.
- **A4**: Cached aligned upstream becomes ahead-by-one; cached ahead-only stays
  ahead-only with one additional commit using the canonical v1 relation string;
  no-upstream becomes untracked; missing cached upstream and
  behind/diverged/unknown states fail.
- **A5**: Linked worktree, wrong/detached branch, expected-head mismatch,
  in-progress Git operation, no staged changes, unstaged/untracked residue,
  signing failure, unsafe receipt path, and prohibited options each return a
  stable typed failure.
- **A6**: Receipt and stdout omit absolute paths, remote URLs, filenames,
  commit-message content, credentials, and signing-key material.
- **A7**: Post-commit invariant or receipt-finalization failure reports the new
  SHA and partial state without automatic history mutation.

### `forge-cli`

- **A8**: A valid receipt bound to the current repository/default branch and
  unchanged remote base is accepted only with fresh direct-main arguments and
  returns the existing verified remote-SHA receipt.
- **A9**: Wrong schema, malformed/oversized/symlink receipt, fingerprint
  mismatch, wrong HEAD/branch, base mismatch, remote movement, unsigned commit,
  dirty checkout, non-single-commit range, URL rewrite, or provider mismatch
  fails before push.
- **A10**: Ordinary `push-default` still rejects the checked-out default branch
  when `--local-default-receipt` is absent.

### Runtime-kit

- **A11**: Exact authorized `semantic-commit local-default` is admitted on the
  expected primary branch; ordinary commit/fixup/squash remains blocked there.
- **A12**: Raw default-branch commit, raw provider push, and covered local ref
  advancement bypasses fail; exact recovery and provider-origin fast-forward
  cleanup continue to pass.
- **A13**: Policy, neutral/Codex/Claude renders, hook manifests, goldens,
  runtime-smoke, and nils-cli surface documentation agree on all three modes.
- **A14**: Local source validation passes without provider credentials or
  network mutation and without moving released version claims.
- **A15**: Final local `main` SHAs and local receipts match in both repositories;
  configured remote refs remain unchanged.

## Validation Plan

### `sympoies/nils-cli`

```bash
cargo test -p nils-semantic-commit
cargo test -p nils-forge-cli repo_push_default
cargo test -p nils-common
cargo test --workspace
```

Use deterministic temporary repositories, local bare remotes, fake Git/provider
runners, isolated signing fixtures, and network-command canaries. Capture a
meaningful failing contract test before production edits and complete the
repository-declared test-first record.

### `agent-runtime-kit`

```bash
scripts/dev/with-nils-version.sh local -- bash tests/agent-hook/run.sh
scripts/dev/with-nils-version.sh local -- bash tests/hooks/run.sh
scripts/dev/with-nils-version.sh local -- bash tests/runtime-smoke/run.sh --mode deterministic
scripts/dev/with-nils-version.sh local -- bash scripts/ci/all.sh
```

Run render/golden updates only for intentional output changes and inspect their
diffs. Record the documentation disposition and declared validation before each
local-main commit. Do not run provider smoke, issue, PR, Actions, release, tap,
or pin-promotion commands during the local-only phase.

## Risks And Guardrails

- **Shared primary checkout**: direct authoring raises collision risk. Guardrail:
  primary-only detection, active checkout lease, clean initial state, exact
  expected HEAD, and no Git operation.
- **Remote presence mistaken for delivery**: a local commit may look finished
  while the provider remains behind. Guardrail: required remote mode, explicit
  receipt fields, local-only terminology, and deploy/release gates unchanged.
- **Stale cached upstream**: cached aligned or ahead-only ancestry may not match
  live provider state. Guardrail: never claim live truth; later push performs a
  fresh live compare-and-swap check, and multi-commit local gaps remain
  ineligible for single-commit receipt adoption.
- **Receipt forgery**: local files are not trusted authorization. Guardrail:
  `push-default` revalidates repository, base, branch, commit shape, signature,
  remote, and provider independently; receipt only selects the governed
  authoring provenance path.
- **Post-commit failure**: signing read-back or receipt finalization can fail
  after Git creates a commit. Guardrail: preflight all possible inputs, return a
  typed partial state, retain the commit, and never auto-reset.
- **Hook bypass surface**: Git has many ref-moving forms. Guardrail: cover the
  common agent paths, keep raw commits/pushes blocked, make CLI checks
  authoritative, and do not describe hooks as a security sandbox.
- **Bootstrap/self-hosting**: the new command cannot authorize its own initial
  implementation before binary and hook support exist. Guardrail: develop on
  managed branches, activate only validated local support, reapply the verified
  diff, and stop rather than bypass if activation cannot be proven.
- **Unreleased local integration**: runtime-kit could claim a CLI surface not
  available from its pinned release. Guardrail: local source validation only;
  release/pin/floor promotion remains deferred.

## Retention Intent

Coordination material. Promote the settled command and policy contract into
`core/policies/git-delivery.md` and `docs/source/nils-cli-surface.md` when the
implementation ships. This capture becomes cleanup-eligible after those
canonical sources, both local implementation receipts, and eventual release
promotion preserve the decisions.

## Read-First References

- `[U1]` Current user requirement: remote-present local mode must be explicitly
  usable; current GitHub spammy classification prohibits issue, PR, and Actions
  use; implementation and this capture should be committed locally.
- `[F1]` `core/policies/git-delivery.md` — current commit, worktree, lease,
  direct-main, and provider-backed cleanup contract.
- `[F2]` `sympoies/nils-cli/crates/semantic-commit/src/cli.rs` and
  `src/commit.rs` — existing command dispatch, semantic commit engine,
  `--expect-head`, `--require-clean`, and JSON result.
- `[F3]` `sympoies/nils-cli/crates/forge-cli/src/ops/repo_push_default.rs` —
  existing provider-bound default-branch push contract.
- `[F4]` `core/hooks/shared/block-unsafe-default-delivery.py` and
  `core/hooks/shared/hook_common.py` — current default-branch and semantic
  command classification.
- `[F5]` `DEVELOPMENT.md` — durable nils-cli ownership, local coupled
  development, validation, and release boundaries.
- `core/policies/work-tier-levels.md` — independent tracking and delivery axes.
- `core/policies/files-hooks-validation.md` — runtime evidence placement and
  hook limitations.

## Recommended Next Artifact

No provider artifact is recommended while GitHub mutations are unavailable.
The next artifact is a local implementation run using this document as its
read-first source, with test-first evidence and local receipts under
`agent-out`. If execution cannot finish in one run, retain the signed managed
branches and create a repo-local plan bundle before resuming; do not substitute
a GitHub issue while the provider remains unavailable.
