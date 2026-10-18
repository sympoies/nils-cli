# forge-cli Spec v1

## Optional identity policy

`identity explain` and `identity doctor` expose the versioned shared
[forge identity contract](../../../nils-common/docs/specs/forge-identity-policy-v1.md).
A configured user policy enforces starting-principal selection and typed refusal
across managed API/Git calls. With no policy file, existing authentication and
Git behavior remain unchanged. The policy contains metadata and credential
reference names only; session binding and environment-specific installation are
separate phases.

## Purpose

This spec is the canonical contract for `forge-cli`, a new binary in the
`nils-cli` workspace. `forge-cli` provides a provider-neutral surface for
the remote forge operations that today are run directly against `gh` and
`glab` from agent-runtime-kit skills (PR/MR lifecycle, Issue lifecycle, CI
wait). Two backends ship together: GitHub (wraps `gh`) and GitLab
(wraps `glab`). Behaviour, validation, and exit semantics are identical
across backends; only the underlying subprocess and the rendered help
text differ.

Goals:

- Replace ad-hoc `gh`/`glab` invocations scattered across agent-runtime-kit
  skills with one binary that enforces branch / body / state policy at
  the type level.
- Manage provider labels from a caller-supplied machine-readable catalog
  without making `forge-cli` the taxonomy source of truth.
- Make the GitHub and GitLab lanes byte-identical from the caller's
  point of view (same flags, same envelope, same exit codes).
- Codify defaults that were previously only described in skill
  Markdown (PR body sections, branch naming, required-check gating,
  draft → ready → merge ordering).
- Stay a thin wrapper: every action delegates to `gh` or `glab` as a
  subprocess. No direct REST client, no extra auth surface, no separate
  rate-limit handling. Auth / SSO / enterprise hosts come from the
  user's existing `gh auth login` and `glab auth login` state.

Non-goals (v1):

- Release deletion and editing, and GitLab releases.
- Arbitrary `gh api` / GitLab REST passthrough — no escape hatch in v1
  on purpose; if a workflow needs it, the call belongs in a focused
  follow-up op, not a generic shim. **Deferred to v2** — see "Open
  questions / v2 candidates" for the re-evaluation criteria. v1
  callers that need a non-CRUD call must keep using `gh api` / `glab
  api` directly from the bash shell until then.
- Issue *macros* beyond create/view/edit/comment/close/reopen — dispatch
  orchestration stays in agent-runtime-kit skills for now.

## Scope

In scope (v1):

- Personal work discovery: `inbox list`, `inbox status`, and
  `inbox next` across GitHub and GitLab.
- Personal activity discovery: `activity commits`, `activity events`, and
  `activity summary`. These v1 personal commands ship GitHub only, with GitLab
  and local backends kept as explicit provider seams for later expansion.
- Repository/project activity discovery: `activity feed` across GitHub and
  GitLab. It normalizes commit and repository/project event surfaces while
  preserving provider-specific event vocabulary.
- PR/MR lifecycle: `create`, `view`, `list`, `edit`, `comment`, `review`,
  native `reviews` read, `ready`, `merge`, `close`.
- PR/MR checks: `pr checks` (one-shot snapshot) and `pr wait-checks`
  (blocking poll until terminal).
- Issue lifecycle: `issue create`, `view`, `edit`, `comment`, `close`,
  `reopen`.
- Program tracker maintenance: `issue tracker lint`, `graph`, and `tick`
  check a tracker issue's phase table, regenerate its dependency graph block,
  and tick one row without rewriting the whole body.
- Repository label lifecycle: `label list`, `label audit`, and
  `label ensure`.
- Repository helpers: read-only `repo view`, the explicitly governed
  `repo push-default` delivery exception, and the GitHub merge-freeze record
  `repo freeze start|end|status`.
- Macro ops: `pr deliver` (kind = `feature` | `bug` | `chore` | `docs` | `ci` | `refactor` | `test`), composing the
  atoms above into the agent-runtime-kit standard "open draft → wait CI →
  ready → merge → cleanup" flow.

Out of scope (v1): inbox mutations, release deletion/editing, label deletion or
rename-by-default, raw REST
passthrough, issue macros, repo creation, branch protection management. `pr
review` posts an outcome comment by default — recording the supplied decision in
the comment without invoking provider-native approval / request-changes state —
and its opt-in `--submit-review` flag submits a native GitHub pull request review
event (`COMMENT` / `APPROVE` / `REQUEST_CHANGES`). With `--thread-file`, it can
also create resolvable GitHub review threads for actionable findings; this stays
GitHub-only because GitLab has no equivalent single review verb/thread mutation
contract in v1. Each remaining out-of-scope item would either widen the parity
gap (GitLab has no equivalent today) or remove the "lock down behaviour" value
(REST passthrough = same as `gh api` + rename).

## Provider parity model

`forge-cli` is a *router + wrapper*, not a client.

- Provider, authority, and repository location are resolved together from
  explicit `--provider` / `--host` / `--repo` inputs and the selected Git
  remote. Unknown or conflicting authorities fail closed as specified under
  "Provider detection"; v1 does not auto-fall-back to HTTPS-only
  Gitea/Forgejo even when the URL shape would permit it.
- All provider API calls go through the backend subprocess. The governed
  `repo push-default` operation additionally invokes local `git` for validation,
  one expected-old-OID compare-and-swap fast-forward, and remote-ref read-back. `forge-cli` does
  not open HTTP sockets, does not hold tokens, and does not write to
  the user's `~/.config/gh` or `~/.config/glab`.
- Backend stdout (typically `--json` from `gh`/`glab`) is parsed and
  re-rendered through the workspace's `cli_contract` envelope. Backend
  stderr is captured; on failure the relevant tail is included in
  `data.error.detail` with secrets redacted (see "Output contract"
  below).
- Backend invocations always force `--json <fields>` (gh) or
  equivalent JSON-bearing flags (glab) where supported. When a `glab`
  subcommand does not support `--json` in the installed version,
  `forge-cli` falls back to text parsing wrapped behind a typed parser
  module so that the brittle bit is isolated.

Parity matrix (v1):

| forge-cli op                                | github backend                                                                                                                               | gitlab backend                                                         | Parity                                                                                                                   |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `pr create`                                 | `gh pr create --draft`; qualified user forks use `<user>:<branch>`                                                                           | `glab mr create --draft`                                               | exact for unqualified heads; qualified heads are a GitHub-only seam                                                      |
| `pr view <id>`                              | `gh pr view <id> --json …`                                                                                                                   | `glab mr view <id> -F json`                                            | exact                                                                                                                    |
| `pr list`                                   | `gh pr list --json …`                                                                                                                        | `glab mr list -F json`                                                 | exact                                                                                                                    |
| `pr edit <id>`                              | `gh pr edit <id> …`                                                                                                                          | `glab mr update <id> …`                                                | exact                                                                                                                    |
| `pr comment <id>`                           | `gh pr comment <id> --body …`                                                                                                                | `glab mr note <id> --message …`                                        | exact                                                                                                                    |
| `pr review <id>`                            | guard `gh api …/pulls/{id}`, then outcome comment, native review, or GraphQL pending review + review thread(s) + submit with `--thread-file` | `glab mr note create <id> --message … --resolvable=false`              | outcome comment, native review event, or resolvable review threads (`--thread-file`, GitHub-only); optional issue mirror |
| `pr review validate [id]`                   | local review body/thread-file validation; with `--check-diff`, `gh api repos/{repo}/pulls/{id}/files`                                        | local validation only; `--check-diff` unsupported in v1                | schema/privacy preflight plus optional GitHub diff-coordinate validation                                                 |
| `pr ready <id>`                             | `gh pr ready <id>`                                                                                                                           | `glab mr update <id> --ready`                                          | exact                                                                                                                    |
| `pr review-threads list <id>`               | `gh api graphql` (`reviewThreads` connection)                                                                                                | `glab api …/merge_requests/<iid>/discussions`                          | normalized thread state                                                                                                  |
| `pr review-threads resolve <id> --thread …` | `gh api` REST reply when a note is supplied, then GraphQL `resolveReviewThread`                                                              | unsupported in v1                                                      | GitHub-only seam                                                                                                         |
| `pr review-threads reply <id> --thread …`   | `gh api` REST review-comment reply                                                                                                           | unsupported in v1                                                      | GitHub-only seam                                                                                                         |
| `pr reviews <id>`                           | `gh api graphql` (native `reviews` connection plus `headRefOid`)                                                                             | unsupported in v1                                                      | GitHub-only normalized current-head/stale review snapshot                                                                |
| `pr pending-review inspect <id> --review …` | PR view + exact-node complete paginated body/inline-comment snapshot                                                                         | unsupported in v1                                                      | GitHub-only receipt-aware read with stable snapshot digest                                                               |
| `pr pending-review resume-submit <id> …`    | exact pending snapshot CAS + `submitPullRequestReview`, or submitted-review read-back                                                        | unsupported in v1                                                      | GitHub-only idempotent recovery for one receipt-bound transaction                                                        |
| `pr pending-review submit <id> …`           | exact pending snapshot CAS + `submitPullRequestReview`                                                                                       | unsupported in v1                                                      | GitHub-only guarded unmarked adoption that preserves inline content                                                      |
| `pr pending-review discard <id> …`          | exact pending snapshot CAS + `deletePullRequestReview`                                                                                       | unsupported in v1                                                      | GitHub-only destructive recovery with distinct inline-content-loss confirmation                                          |
| `pr pending-review delete <id> --review …`  | PR view + complete pending-only membership snapshot + exact-target final read with content/viewer/comment guards + `deletePullRequestReview` | unsupported in v1                                                      | GitHub-only compatibility recovery for one confirmed-abandoned body-only pending review                                  |
| `pr review-dismiss <id> --review-id …`      | PR view + REST review read + designated-PASS admission + `gh api user` + `PUT …/reviews/{id}/dismissals`                                     | unsupported (no review dismissal)                                      | GitHub-only governed dismissal of one configured automated CHANGES_REQUESTED review                                      |
| `pr tasks <id>`                             | `gh pr view <id> --json number,url,body`                                                                                                     | `glab mr view <id> -F json` (`description`)                            | normalized task-list state                                                                                               |
| `pr merge <id>`                             | `gh pr merge <id> --squash --delete-branch`                                                                                                  | `glab api --method PUT .../merge` after gates                          | exact (method honoured per repo cfg)                                                                                     |
| `pr close <id>`                             | `gh pr close <id>`                                                                                                                           | `glab mr close <id>`                                                   | exact                                                                                                                    |
| `pr checks <id>`                            | `gh pr checks <id> --json …` plus `--required` for gating                                                                                    | `glab mr view -F json` + `glab api .../pipelines/<id>/jobs`            | emulated on GitLab                                                                                                       |
| `pr wait-checks <id>`                       | poll exact-head REST checks against configured base-branch requirements                                                                      | poll structured MR pipeline/jobs snapshot                              | emulated; same envelope                                                                                                  |
| `issue create`                              | `gh issue create …`                                                                                                                          | `glab issue create …`                                                  | exact                                                                                                                    |
| `issue view <id>`                           | `gh issue view <id> --json …`                                                                                                                | `glab issue view <id> -F json`                                         | exact                                                                                                                    |
| `issue edit <id>`                           | `gh issue edit <id> …`                                                                                                                       | `glab issue update <id> …`                                             | exact                                                                                                                    |
| `issue comment <id>`                        | `gh issue comment <id> --body …`                                                                                                             | `glab issue note <id> --message …`                                     | exact                                                                                                                    |
| `issue close <id>`                          | `gh issue close <id>`                                                                                                                        | `glab issue close <id>`                                                | exact                                                                                                                    |
| `issue reopen <id>`                         | `gh issue reopen <id>`                                                                                                                       | `glab issue reopen <id>`                                               | exact                                                                                                                    |
| `issue tracker lint <id>`                   | `issue view`, plus one `issue view` per referenced issue with `--check-state`                                                                | same                                                                   | exact (grammar is provider-neutral)                                                                                      |
| `issue tracker graph <id>`                  | `issue view`; with `--write`, `issue edit --body`                                                                                            | `issue view`; with `--write`, `issue update --description`             | exact                                                                                                                    |
| `issue tracker show <ref>`                  | `issue view`                                                                                                                                 | `issue view`                                                           | exact                                                                                                                    |
| `issue tracker tick <id>`                   | `issue view` + `issue edit --body`; with `--comment-file`, the `issue comment` call                                                          | `issue view` + `issue update --description`; `issue note`              | exact                                                                                                                    |
| `label list`                                | `gh label list --json …`                                                                                                                     | paged `glab label list --output json --per-page … --page …`            | exact                                                                                                                    |
| `label audit`                               | read labels, compare with caller catalog                                                                                                     | same                                                                   | exact                                                                                                                    |
| `label ensure`                              | `gh label create/edit`                                                                                                                       | `glab label create/edit`                                               | exact (no delete/rename by default)                                                                                      |
| `auth status`                               | `gh auth status --hostname <authority>`                                                                                                      | `glab auth status --hostname <authority>`                              | exact (text → typed)                                                                                                     |
| `repo view`                                 | `gh repo view <slug-or-host/slug> --json …`                                                                                                  | `glab repo view <slug-or-https-url> -F json`                           | exact                                                                                                                    |
| `repo push-default`                         | local Git validation/push plus host-qualified `gh repo view --json …` default-branch resolution                                              | local Git validation/push plus host-qualified `glab repo view -F json` | exact; no provider force path                                                                                            |
| `repo freeze` (start, end, status)          | `gh label list/create`, `gh issue create/list/close` on the `merge-freeze` label                                                             | `provider_unsupported`                                                 | GitHub-only in v1                                                                                                        |
| `inbox list`                                | `gh search prs/issues --json …`                                                                                                              | `glab api --hostname <host> …`                                         | normalized aggregation                                                                                                   |
| `inbox status`                              | same provider reads as `inbox list`                                                                                                          | same provider reads as `inbox list`                                    | bounded counts                                                                                                           |
| `inbox next`                                | same provider reads as `inbox list`                                                                                                          | same provider reads as `inbox list`                                    | ranked bounded subset                                                                                                    |
| `activity commits`                          | `gh api search/commits`                                                                                                                      | unsupported in v1                                                      | GitHub-only seam                                                                                                         |
| `activity events`                           | `gh api users/<login>/events[/public]`                                                                                                       | unsupported in v1                                                      | GitHub-only seam                                                                                                         |
| `activity feed`                             | `gh api repos/<owner>/<repo>/commits` + repository activity REST                                                                             | `glab api projects/:id/repository/commits` + project events            | normalized open vocabulary                                                                                               |
| `activity summary`                          | `gh api graphql` contribution query                                                                                                          | unsupported in v1                                                      | GitHub-only seam                                                                                                         |
| `pr deliver`                                | macro: exact user/repository-aware lookup → `pr create` or adopt → `pr wait-checks` → `pr ready` → `pr merge`                                | branch lookup → same remaining composition                             | exact for unqualified heads; qualified user-fork lookup is a GitHub-only seam                                            |

"emulated" means the backend's native command differs in shape, but
`forge-cli` normalises both into the same `cli.forge-cli.pr.checks.v1`
payload so callers can branch on `data.state` regardless of host.

GitLab capability status:

- Supported with stable JSON/API: PR/MR create, view, list, edit, comment,
  ready, close, checks, wait-checks, merge, deliver; issue create/view/edit,
  comment, close, reopen, tracker lint/graph/tick; label list/audit/ensure;
  auth status; repo view and
  governed repo push-default;
  inbox list/status/next; activity feed.
- Intentionally unsupported in v1: GitLab `activity commits`,
  `activity events`, `activity summary`, and `search` operations.
- Fragile fallback: branch-only `pr checks <branch>` / `pr wait-checks
  <branch>` without a repo/project path uses `glab ci status -b <branch>` text
  parsing and keeps the `glab_version_unsupported` guard. Numeric MR lifecycle
  paths use structured MR pipeline/jobs API data and do not depend on the text
  parser minor range.

## Naming and topology

- Crate: `nils-forge-cli` (matches the `nils-*` package convention).
- Binary: `forge-cli`.
- Library entry: `crates/forge-cli/src/lib.rs`.
- Wrapper (Homebrew formula bin): `wrappers/forge-cli`.
- Completions: `completions/zsh/_forge-cli`, `completions/bash/forge-cli`.

Command tree:

```text
forge-cli
├── pr
│   ├── create
│   ├── view
│   ├── list
│   ├── edit
│   ├── comment
│   ├── review
│   │   └── validate
│   ├── review-threads
│   │   ├── list
│   │   ├── resolve
│   │   └── reply
│   ├── reviews
│   ├── pending-review
│   │   ├── inspect
│   │   ├── resume-submit
│   │   ├── submit
│   │   ├── discard
│   │   └── delete
│   ├── review-dismiss
│   ├── ready
│   ├── merge
│   ├── close
│   ├── checks
│   ├── wait-checks
│   └── deliver           (macro)
├── issue
│   ├── create
│   ├── view
│   ├── edit
│   ├── comment
│   ├── close
│   ├── reopen
│   └── tracker
│       ├── lint
│       ├── graph
│       └── tick
├── activity
│   ├── commits
│   ├── events
│   └── summary
├── label
│   ├── list
│   ├── audit
│   └── ensure
├── inbox
│   ├── status
│   ├── list
│   └── next
├── repo
│   ├── view
│   └── push-default
├── auth
│   └── status
└── completion              (workspace standard)
```

Global flags (every subcommand):

- `--format text|json` (workspace standard).
- `--remote <name>` (default `origin`).
- `--provider github|gitlab|local` (override auto-detect).
- `--host <hostname[:port]>` — select a canonical forge authority. The input
  is an authority only: schemes, userinfo, paths, query strings, fragments,
  whitespace, control characters, malformed DNS labels, and invalid ports are
  rejected. HTTPS port `443` is normalized away; other ports are retained.
  With `--provider`, a valid custom authority is accepted unless it is
  positively identified as the other provider. Without `--provider`, the host
  must identify a supported provider.
- `--repo <path>` (override the remote-derived repository path). Repository-
  scoped commands validate `owner/name` for GitHub,
  `group[/subgroup...]/project` for GitLab, and a non-empty local-compatible
  path for Local; invalid paths return `DATA 65` with
  `error.kind=repo_invalid`. Repository-independent commands such as `auth
  status` and personal `activity` commands do not reject an otherwise
  irrelevant repository shape.
- `--dry-run` — render the backend command that *would* run plus all
  validation checks, but do not invoke it. Output envelope carries the
  exact argv under `data.plan` for atomic commands or `data.actions[].plan`
  for `label ensure`. `repo push-default` instead runs its read-only local and
  provider preflight and emits `pushed=false` with the exact `push_refspec`;
  it never invokes `git push` in dry-run mode. `issue tracker graph --write`
  and `issue tracker tick` likewise perform their provider read, so the planned
  change can be shown, and emit `written=false` with the write and comment
  calls under `data.actions[].plan`; they never invoke those calls in dry-run
  mode.

Every backend subprocess is bound to the resolved authority for that call.
`forge-cli` removes ambient `GH_HOST` and `GITLAB_HOST` from the child
environment, then sets the selected provider's variable (`GH_HOST` or
`GITLAB_HOST`) explicitly, so unrelated shell configuration cannot retarget a
request.

Inbox-local flags:

- `forge-cli inbox list|status --limit <n>` bounds each provider query family
  (default `30`).
- `forge-cli inbox next --limit <n>` bounds the returned ranked candidates
  (default `5`); provider reads still use at least `30` candidates.
- `--kind review|assigned|todo|authored|involved` is repeatable and selects
  inbox *reasons* (why an item appears). `involved` is opt-in because it can
  be broad on GitHub.
- `--item-type all|pr|issue` selects *result classes* (pull/merge requests vs
  issues) and is distinct from `--kind`. Default is `all`. PR-only mode skips
  GitHub issue searches and GitLab issue API calls; issue-only mode skips PR
  searches (GitHub review-requested is dropped). GitLab `todos` are kept when
  their `target_type` (or target URL) matches the selected item type;
  unclassifiable todos appear only in `all` mode.
- Global `--host` is rejected for `inbox list|status|next` with an actionable
  `provider_unsupported` error. Inbox has a separate multi-provider resolver;
  use inbox-local `--gitlab-host` for a non-default GitLab authority. GitHub
  Enterprise inbox search is unsupported in v1.
- `--gitlab-host <host>` is scoped to inbox commands only and is passed to
  every GitLab `glab api` invocation as `--hostname <host>`.
- When `--gitlab-host` is omitted, GitLab host resolution uses
  `FORGE_CLI_INBOX_GITLAB_HOST`, then GitLab remote inference, then
  `gitlab.com`.
- With no `--provider`, inbox queries both default providers. `--provider
  github|gitlab` narrows the inbox just as it narrows lifecycle commands, but
  inbox does not reuse the single-provider remote resolver internally.
- `--gitlab-vpn off|optional|required` controls an inbox-local GitLab
  readiness gate. `required` runs a readiness check before any GitLab backend
  call and reports `vpn_unavailable` without invoking `glab` when the check
  fails. `optional` may report a warning but still attempts GitLab. `off` is
  the default.
- `--gitlab-vpn-check tcp:<host>:<port>|cmd:<program>|openvpn` selects the
  readiness check. `tcp:` probes reachability, `cmd:` delegates to a local
  operator script, and `openvpn` verifies local OpenVPN CLI/profile
  prerequisites only; `forge-cli inbox` never starts or stops a VPN.
- `--gitlab-openvpn-profile <path>` is local-only input for probes. Profile
  paths are redacted from dry-run JSON, warnings, provider errors, cache files,
  issue records, and docs examples.
- `--gitlab-vpn-check-timeout <duration>` bounds readiness checks (default
  `2s`). `--provider-timeout <duration>` bounds GitLab inbox identity/API
  backend calls (default `20s`; `0s` disables). GitHub queries remain
  independent from GitLab timeout behavior.
- `--strict-providers` makes partial provider failure a non-zero
  `provider_failed` failure envelope with `error.details.providers[]`.
- `--cache-fallback` includes recent stale cached provider items when a
  selected provider is VPN-unavailable or times out. Cache fallback is opt-in,
  age-bounded by `--cache-max-age` (default `30m`), and disabled with
  `--no-cache`.

Activity-local flags:

- `forge-cli activity commits --user <login|@me> --since <date-or-datetime>
  --limit <n>` searches recent commits authored by the selected GitHub user.
- `forge-cli activity events --user <login|@me> --public-only --limit <n>`
  lists recent user events. Without `--public-only`, `@me` can use the
  authenticated user's event endpoint.
- `forge-cli --repo <owner/name|group/project> activity feed
  --since <date-or-datetime> --limit <n>` lists recent repository/project
  activity. GitHub reads commits plus repository activity; GitLab reads
  commits plus project events.
- `forge-cli activity summary --user <login|@me> --since <date-or-datetime>
  --limit <n>` summarizes commit contributions by repository.
- Activity limits are effective provider page sizes and are clamped to the v1
  per-page maximum of `100`.
- `@me` resolves through `gh api user`; `--dry-run` includes both the identity
  lookup and the data query under `data.plans[]`.

Label-local flags:

- `forge-cli label list --limit <n>` reads at most `<n>` provider labels and
  emits `cli.forge-cli.label.list.v1`. The limit is a provider-neutral total;
  GitLab advances `--page` with a maximum `--per-page 100` until the total is
  reached or a short page proves the catalog is exhausted.
- `forge-cli label audit --catalog <path> --limit <n>` compares provider
  labels to the caller-owned catalog and reports missing labels, color /
  description drift, and unknown shared labels.
- `forge-cli label ensure --catalog <path> [--update-existing]` creates
  missing labels and updates existing color / description drift only when
  explicitly requested. It never deletes or renames labels by default.
- `pr create` and `pr deliver` accept `--label <name>` repeatedly. Catalog
  validation is opt-in via `--label-catalog <path> --strict-labels`.
  The compatibility `state::do-not-merge` hold may coexist with one ordinary workflow
  `state::` label. It must still be known (or an allowed extension) and applicable
  to the target. Two ordinary states still conflict in an exclusive state group,
  with or without the hold. This shared selection rule applies to typed issue,
  PR, and MR label validation; it does not lift merge holds or change the catalog.

## Atomic op surface

The full machine-readable list lives in
[`forge-cli-ops-v1.yaml`](forge-cli-ops-v1.yaml). The linked operation contracts
below are the human-readable reference; the YAML is authoritative for
backend mapping, validation rules, and output schema versions.

### `release create` / `release upload`

See [publication and administration](forge-publication-admin-v1.md#release-create--release-upload) for this contract.

### `workflow dispatch`

See [publication and administration](forge-publication-admin-v1.md#workflow-dispatch) for this contract.

### `comment edit` / `comment delete`

See [publication and administration](forge-publication-admin-v1.md#comment-edit--comment-delete) for this contract.

### `pr create`

- Input: `--head <branch>` (default current branch; GitHub also accepts
  `<user>:<branch>` for a cross-fork head), `--base <branch>`
  (default repo default branch), `--title <str>`, `--body-file <path>`
  or `--body <str>`, `--kind feature|bug|chore|docs|ci|refactor|test`, `--draft` (default `true`),
  `--reviewer <user>...`, `--label <name>...`,
  `--label-catalog <path>`, `--strict-labels`.
- Validation (see "Lock-down policy" for the full list):
  - the branch, or the branch suffix of a qualified GitHub head, MUST match
    the semantic branch-name rule and align with `--kind`;
  - a qualified GitHub user MUST own the local branch's single upstream push
    repository on the selected GitHub authority; the full `<user>:<branch>`
    ref is preserved for provider dispatch, while local branch checks consume
    only the suffix;
  - immediately after creation, GitHub's `headRefName`, `headRefOid`, and
    `headRepository.nameWithOwner` MUST match the local branch, local commit,
    and upstream repository before success is reported;
  - title length ≤ 70 chars, no trailing whitespace;
  - body MUST contain non-empty configured H2 sections (defaulting to
    `## Summary` and `## Test plan`);
  - working tree MUST be clean (`git status --porcelain` empty);
  - resolved local head branch MUST be pushed and remote-tracked.
- Output schema: `cli.forge-cli.pr.create.v1`,
  `data = { number, url, head, head_sha?, head_repository?, base, draft,
  title, kind, provider }`.

Qualified heads are deliberately a GitHub-only user-fork seam. GitHub CLI's
`gh pr create --head` contract accepts `<user>:<branch>` but does not support
an organization as the qualifier. `forge-cli` therefore does not advertise
`<owner>:<branch>` or organization-fork support. It validates only the
non-empty, single-qualifier structure locally, so provider-valid Enterprise
Managed User logins (including underscore suffixes) reach GitHub; provider
rejection remains authoritative for username grammar and account-type
distinctions that cannot be proven offline.

### `repo push-default`

See [default-branch publication](forge-default-push-v1.md#repo-push-default) for this contract.

### `pr wait-checks`

See [checks and review reads](forge-checks-review-reads-v1.md#pr-wait-checks) for this contract.

### `pr review-threads` (read)

See [checks and review reads](forge-checks-review-reads-v1.md#pr-review-threads-read) for this contract.

### `pr reviews` (read)

See [checks and review reads](forge-checks-review-reads-v1.md#pr-reviews-read) for this contract.

### Review transaction state and pending-review recovery

See [review transaction and handoff state](forge-review-state-v1.md#review-transaction-state-and-pending-review-recovery) for this contract.

### `pr review-handoff assign` / `inspect` / `check` / `surrender` / `recover`

See [review transaction and handoff state](forge-review-state-v1.md#pr-review-handoff-assign--inspect--check--surrender--recover) for this contract.

### `pr review-dismiss`

See [review publication and thread writes](forge-review-publication-v1.md#pr-review-dismiss) for this contract.

### `pr review-loop inspect` / `observe` / `extend` / `validate`

See [review transaction and handoff state](forge-review-state-v1.md#pr-review-loop-inspect--observe--extend--validate) for this contract.

### `pr pending-review delete` compatibility surface

See [review transaction and handoff state](forge-review-state-v1.md#pr-pending-review-delete-compatibility-surface) for this contract.

### `pr review`

See [review publication and thread writes](forge-review-publication-v1.md#pr-review) for this contract.

### `pr review validate`

See [review publication and thread writes](forge-review-publication-v1.md#pr-review-validate) for this contract.

### `pr review-threads resolve` / `pr review-threads reply`

See [review publication and thread writes](forge-review-publication-v1.md#pr-review-threads-resolve--pr-review-threads-reply) for this contract.

### `pr merge`

See [pull request merge](forge-merge-v1.md#pr-merge) for this contract.

### `label audit` / `label ensure`

See [publication and administration](forge-publication-admin-v1.md#label-audit--label-ensure) for this contract.

### `issue tracker lint` / `graph` / `tick`

See [publication and administration](forge-publication-admin-v1.md#issue-tracker-lint--graph--tick) for this contract.

## Macro: `pr deliver`

`pr deliver` is the canonical end-to-end flow agent-runtime-kit's
`deliver-{feature,bug}-pr` skills compose today. It is implemented in
Rust so behaviour is fixed at the type level and identical across
providers.

Synopsis:

```text
forge-cli pr deliver \
  --kind feature|bug|chore|docs|ci|refactor|test \
  [--title <str>] [--body-file <path>] \
  [--base <branch>] [--head <branch>] \
  [--method squash|merge|rebase] \
  [--reviewer <user>...] \
  [--label <name>...] \
  [--label-catalog <path> --strict-labels] \
  [--timeout <duration>] \
  [--review-convergence[=<true|false>]] \
  [--no-merge]        # stop after wait-checks; useful in CI
```

Steps:

Before any step, `--strict-labels` validates `--label-catalog` and every
selected label against the provider target. This same pure preflight runs for
dry-run, create, and adopt delivery, so `label_catalog_missing` and related
label errors are returned before provider access in every mode.

1. `auth status` — fail-fast on missing auth.
2. `repo view` — resolve default branch, repo slug, default merge
   method override.
3. Exact head-and-base lookup (`pr list --state open --head <branch> --base
   <resolved-base>`) — when an
   open PR/MR already exists for the resolved head branch, the macro
   adopts it instead of creating: the create step (and its create-input
   gates) is skipped, an `adopt` step carrying the PR's `pr.view`
   payload is recorded, and the lifecycle resumes from step 5. The
   adopted PR's *actual* body (fetched via `pr view`) is re-validated
   against the body-section gate, and the branch-kind / clean-worktree /
   resolved-head push-state rules still apply. `--title` / `--body`
   inputs are ignored on adoption — the existing PR keeps its own.
   For a qualified GitHub `<user>:<branch>` head, `gh pr list --head` is not
   given the unsupported qualified syntax. Delivery lists a bounded open set
   and adopts only a row whose branch and `headRepository` exactly match the
   locally bound user/upstream repository; a same-named branch from any other
   fork is ignored. A saturated bounded result without that exact row fails
   closed instead of assuming absence. The subsequent `pr view` must still
   match branch, repository, local SHA, and the exact resolved base. The same
   base is revalidated after checks, after ready, and by merge;
   `--allow-non-default-base` authorizes only this value.
4. `pr create --draft` — atom; validates branch / title / body. Only
   runs when the lookup found nothing.
5. `pr wait-checks` — atom; blocks until terminal within one cumulative
   `--timeout` budget. Delivery uses the atom's required-only wait, so on
   GitHub a head with zero required checks and visible check rows is gated
   on all visible checks: pending visible checks keep it polling and failed
   visible checks block delivery. GitLab behavior is unchanged.
6. `pr ready` — atom; only if previous step is `success`.
7. `pr merge` — atom; honours `--method`, repo override, and the same resolved
   review-convergence policy as a direct merge. `--no-merge` performs no
   convergence wait because the merge phase is skipped.
8. Emit single envelope summarising every sub-step output under
   `data.steps[]` (`create` and `adopt` are mutually exclusive). The
   macro's own schema is `cli.forge-cli.pr.deliver.v1`.

Failure semantics:

- A failing step short-circuits the macro. The envelope still lists
  every step attempted, with the failing one's `ok = false` and the
  remaining ones omitted (not present in the array).
- The macro's outer exit code is the failing atom's exit code, not
  remapped. So `checks_failed` propagates as `RUNTIME 1`, `policy`
  violations propagate as `DATA 65`, and so on.

## Lock-down policy

These rules are enforced regardless of which backend runs. They are
declared in `forge-cli-ops-v1.yaml` (`validations:` per op) so the
backend implementations cannot diverge.

1. **Branch naming.** Feature work MUST be on `feat/<slug>`, bug work
   on `fix/<slug>`. Slug is `[a-z0-9][a-z0-9-]{1,63}`. Ticket prefix
   `abc-123-` is allowed inside the slug. `--kind` and the branch
   prefix MUST agree.
2. **Body schema.** PR/MR body MUST contain non-empty H2 sections named by
   `[body].summary_heading` and `[body].test_plan_heading`. Defaults are
   `## Summary` and `## Test plan`. Names match exactly, including the `##`
   prefix and following space; no synonyms are inferred. Order is not enforced; presence and
   non-emptiness are. The same resolved headings apply to `pr create`, body
   replacement in `pr edit`, `pr deliver --dry-run` preflight, and the actual
   provider body when `pr deliver` adopts an existing PR.
3. **Title length.** ≤ 70 characters, no trailing whitespace, no
   trailing punctuation other than `?`.
4. **Working tree.** `git status --porcelain` MUST be empty for
   `create`, `merge`, and `deliver`. (`view`/`list`/`comment` are
   read/append-only and do not check.)
5. **Push state.** The resolved head branch MUST be pushed to a
   remote-tracking branch before `create` or `deliver` runs.
6. **Default-branch protection.** PR/MR delivery remains the default.
   `forge-cli` refuses any unconstrained force-push, caller-controlled lease,
   delete, or direct merge
   into the repo default branch. The sole direct-delivery exception is
   `repo push-default`: one clean, locally verified signed commit on the exact
   expected remote base, one uniquely bound actual push destination, delivered
   with an exact old-OID compare-and-swap fast-forward and remote SHA read-back.
   `--allow-non-default-base` applies only to PR/MR base branches; no flag
   bypasses the default-branch force refusal.
7. **Draft → ready → merge ordering.** `pr merge` refuses to merge a
   draft. There is no `--merge-as-draft`. Callers MUST run `pr ready`
   first (or use `pr deliver`, which sequences them).
8. **Required-check gating.** `pr merge` re-checks required-check
   state immediately before invoking the backend, even if
   `pr wait-checks` was called earlier in the macro. This is the
   TTL-zero re-check that addresses the
   `github-pr-required-check-gating` operation record.

   **Absence is not success.** "All required checks passed" is vacuously
   true over an empty set, so a snapshot reporting no required checks
   *and* no visible check rows means nothing ran — not that everything
   passed. `pr merge` fails closed with `checks_not_registered`
   (`DATA 65`) and `pr wait-checks` treats the empty set as non-terminal,
   polling out its budget before reporting the same kind. Both accept
   `--allow-no-checks` (with a recorded `--allow-no-checks-reason`) for a
   repository that genuinely configures none.

   The predicate deliberately requires *both* halves. A repository can run
   CI without branch protection, producing zero required checks alongside
   visible rows; refusing those would block every such repository. What is
   refused is the case with nothing to fall back to, which is also what the
   provider reports during the check-registration window after a
   force-with-lease — the window in which delivery previously reported
   success against an unchecked head.

   Be precise about what happens to the zero-required-with-visible-rows
   case, because the two commands differ. `pr deliver` re-gates those rows
   against the full visible set — but only on GitHub
   (`should_gate_visible_checks`). Standalone `pr merge` does **not**
   re-gate them on any provider: its snapshot is taken with
   `required_only`, so a non-required failing row is not in `failed` and
   the gate accepts the head. That gap predates this rule and is not
   closed by it.

   The rule is provider-neutral, and on GitLab that is a behaviour change
   worth stating: an MR whose head has no pipeline — no `.gitlab-ci.yml`,
   excluded by `workflow:rules`, or pipelines disabled on a fork — reports
   the same empty snapshot and is now refused rather than merged. That is
   the intended reading of "absence is not success", and
   `--allow-no-checks`, or `[checks] none = true` with a `none_reason` in
   `.forge-cli.toml`, is the declared way to say a project has no CI.
9. **Merge method.** Default `squash`. Repo override allowed via
   `.forge-cli.toml` `[merge] method = "squash" | "merge" | "rebase"`.
   Per-invocation `--method` overrides the repo override; both are
   logged in `data.method`.
10. **Branch cleanup.** `pr merge` deletes the remote branch by
    default (`--delete-branch` on `gh`, `--remove-source-branch` on
    `glab`). Disable with `--keep-branch`. Local branch cleanup is
    out of scope for `forge-cli` and remains the bash wrapper's job
    in agent-runtime-kit skills.
11. **Portable paths.** PR/MR and issue title, body, and comment text MUST
    NOT embed a machine-local home path (`/Users/<owner>/…`,
    `/home/<owner>/…`). This mirrors the repo-side portable-paths file hook on
    the provider egress path so local paths cannot leak into provider content.
    The error `detail` enumerates each offending line and its `$HOME`-relative
    fix without echoing the original personal path. Literal container /
    CI-runner home roots (`/home/agent`, `/home/linuxbrew`, and the CI runner
    work root) are allowlisted — see the
    `no_local_path` entry in `forge-cli-ops-v1.yaml` for the exact list; set
    `FORGE_CLI_ALLOW_LOCAL_PATH=1` to bypass a verified false positive. Enforced
    by `pr create`, `pr edit`, `issue create`, `issue edit`, `pr comment`,
    `pr review`, and `issue comment`; `issue tracker graph --write` and
    `issue tracker tick` write through the `issue edit` and `issue comment`
    paths and inherit it.
12. **Opt-in native-review convergence.** `pr merge` and the merge step of
    `pr deliver` resolve `--review-convergence[=true|false]` over repo and
    global config. The default is off. In v1, configured bots use `observed`:
    an absent bot never waits or fails, while already-observed current-head
    review, live non-outdated thread, or unowned comment activity must stay quiet
    for `quiet_period`. New semantic activity restarts that window; marker-only
    forge-cli disposition replies, empty `COMMENTED` reviews, provider ids and
    timestamps, and resolve/outdated transitions do not. Reopening a live thread
    does restart the window. `timeout` bounds the complete active wait, including provider
    calls and rate-limit waits. Polling occurs at most once every 10 seconds.
    Current-head native `CHANGES_REQUESTED` fails with
    `review_changes_requested`; `COMMENTED`
    prose is never parsed as a verdict. Older-head reviews are returned as
    stale evidence. A head change during an active wait fails closed with
    `review_convergence_head_changed`; a missing initial provider head fails
    with `review_convergence_head_missing` before review collection. The
    complete paginated review plus thread/comment snapshot is read
    again immediately before merge; any digest drift fails with
    `review_convergence_activity_changed`, and incomplete or unknown provider
    data fails with `review_snapshot_incomplete`. Every review must carry a
    non-empty `commit.oid` before current-head/stale classification. For each
    reviewer, a later
    native `APPROVED` or `DISMISSED` supersedes an earlier request on the same head;
    `COMMENTED` does not alter that opinionated state. GitHub is the only
    supported provider in v1; an enabled policy elsewhere returns
    `provider_unsupported`.
13. **Review-thread gating.** `pr merge` (and the `pr deliver` merge
    step) fetches review threads during the final merge gates and refuses to
    merge while any **non-outdated** thread is unresolved
    (GitHub `reviewThreads`, GitLab resolvable discussions). An **outdated**
    unresolved thread (its anchored diff hunk changed) is mechanically
    dispositioned `stale` — recorded in the merge payload as
    `stale_thread_dispositions` (thread id, author, path, summary, rationale),
    never silently dropped — and no longer blocks, so an accumulation of stale
    bot threads can no longer wedge convergence. Bypass any remaining live
    threads with `--allow-unresolved-threads` plus a required
    `--allow-unresolved-threads-reason`, recorded as
    `unresolved_threads_override_reason`. The local provider has no
    thread model and passes trivially. The error `detail` lists each
    blocking (non-outdated) unresolved thread (author, file anchor, first line).
14. **Task-list gating.** `pr merge` (and the `pr deliver` merge step)
    parses GFM task-list items out of the PR/MR description fetched at
    merge time and refuses to merge while any `- [ ]` item is
    unchecked (`- [x]` / `- [X]` count as done, GitLab's `- [~]` as
    inapplicable; fenced code blocks are skipped). Bypass with
    `--allow-unchecked-tasks` plus a required
    `--allow-unchecked-tasks-reason`, which is recorded in the merge
    payload as `unchecked_tasks_override_reason`. Providers without a
    body model (local) pass trivially. The error `detail` lists each
    unchecked item (line number, text).
15. **Review-thread write ownership.** `pr review-threads reply` and
    `pr review-threads resolve` verify live GitHub writes by fetching the
    positional PR's review threads and confirming `--thread <id>` is present
    before posting a reply or resolving. `--dry-run` remains offline and skips
    this lookup.
16. **Pending-review recovery ownership and content binding.** `inspect`,
    `resume-submit`, guarded unmarked `submit`, and `discard` read the exact
    `--review` node plus every inline-comment page and bind mutations to the
    resulting snapshot digest, PR head, review commit, PR membership, and
    provider-native viewer identity. Receipt-bound automatic recovery adds only
    a missing ordered manifest suffix and never deletes a draft; an
    already-submitted matching run is idempotent success. Unmarked submit and all
    discard paths require explicit confirmation, with a distinct confirmation
    for inline-content loss. The compatibility `delete` additionally requires
    `--confirm-abandoned` and an exact bounded body/body-file value, retains only
    the target during complete membership pagination, re-fetches the exact node,
    and refuses any inline comment. These provider-native viewer fields support
    both user and GitHub App installation actors without `GET /user`. GitHub
    exposes no content CAS on delete/discard, so the documented
    final-read-to-delete race remains after exact snapshot guards.
17. **No agent attribution.** PR/MR and issue title, body, and comment text
    MUST NOT carry agent self-attribution: a generator marker line
    (`Generated with …` prose, or a `claude.com/claude-code` /
    `claude.ai/code` link) or a `Co-Authored-By` trailer whose value names the
    model family or carries the vendor no-reply address
    (`noreply@anthropic.com`). The marker forms are defined once in
    `nils_common::agent_attribution` and shared with `semantic-commit`'s
    `claude-coauthor-trailer` / `claude-generated-marker` blocked-message rules,
    so the commit path and the provider path cannot diverge. Enforcement lives
    in the CLI rather than in an agent-harness hook, so the rule holds whether or
    not the calling runtime declares a matching hook of its own. Text *about*
    the rule is allowed: fenced
    code blocks and inline code spans are stripped before the scan, so a body
    documenting `` `Co-Authored-By: Claude ...` `` passes while a bare
    attribution line does not (commit messages get no such exemption — that
    scan is verbatim). The error `detail` enumerates each offending line and its
    fix without echoing the marker; set
    `FORGE_CLI_ALLOW_AGENT_ATTRIBUTION=1` to bypass a verified false positive.
    Enforced by `pr create`, `pr edit`, `issue create`, `issue edit`,
    `pr comment`, `issue comment`, `pr review`, `pr review-threads reply`, and
    `pr review-threads resolve`; the `issue tracker` writes inherit it from
    `issue edit` and `issue comment`.
18. **Merge freeze and merge queue (GitHub).** Right after the repository
    read, `pr merge` issues one GraphQL policy read (`ForgeMergePolicy`): the
    open issues labelled `merge-freeze`, the base branch's `mergeQueue`, and the
    PR node id, queue membership, and state. The read is repeated after
    review-convergence rechecks and before the final hold, stacked-branch, and
    review-loop tip checks preceding merge or enqueue, and the freeze gate runs
    again with the same overrides. A freeze started while the gates ran therefore
    still blocks.
    - A failed or partial read fails closed with `merge_policy_unavailable`
      (`UNAVAILABLE 69`). One case is not a failure: a host whose schema has no
      merge queue (an older GitHub Enterprise Server) reports that `mergeQueue`
      or `isInMergeQueue` does not exist. It is treated as having no queue, and
      the freeze list alone is read (`ForgeMergeFreezes`).
    - An open `merge-freeze` issue is an active freeze. The merge is refused
      with `merge_freeze_active`, whose detail names each freeze (number, title,
      author, start, URL). The freeze holder bypasses it with one
      `--allow-merge-freeze <issue>` per active freeze plus a required
      `--allow-merge-freeze-reason`; the bypass is recorded as
      `merge_freeze_override`. A freeze lives on the provider, so it applies to
      private repositories whose plan offers no branch protection or merge
      queue. `repo freeze start|end|status` manages the record; ending one that
      is not open is `merge_freeze_not_active`, and ending without `--issue`
      while several are open is `merge_freeze_ambiguous`.
    - When the base requires a merge queue, the direct merge API is not used.
      After every other gate passes, the verified head is enqueued with
      `enqueuePullRequest` and `expectedHeadOid`. Enqueue is skipped when the PR
      is already queued or already merged, so a retried merge resumes the wait
      instead of failing. The command then polls until the PR is merged,
      bounded by `--queue-timeout` (default 2700 s). Failures:
      - `merge_queue_enqueue_rejected` (`RUNTIME 1`): the enqueue mutation
        returned GraphQL errors or no queue entry.
      - `merge_queue_dequeued` (`RUNTIME 1`): the PR was closed, or stayed open
        without a queue entry through the whole 60 s grace. The provider can
        drop the entry of a PR it merged before the PR reads as merged, so
        that reading is re-polled first; when `--queue-timeout` ends the wait
        inside the grace, the result is `merge_queue_timeout`. A PR that reads
        merged before its merge commit is reported is re-polled through the
        same grace, bounded by the timeout, then its commit is read from the
        PR view.
      - `merge_queue_checks_failed` (`RUNTIME 1`): the entry was reported
        unmergeable.
      - `merge_queue_poll_failed` (`UNAVAILABLE 69`): a poll returned GraphQL
        errors.
      - `merge_queue_timeout` (`UNAVAILABLE 69`): the bound was exceeded. The
        PR is still queued and will merge unless it is dequeued, so rerunning
        `pr merge` resumes the wait.
    - The queue's configured method wins. An explicit `--method` that differs is
      refused with `merge_queue_method_mismatch` before enqueue; `pr deliver`
      without `--method` lets the queue decide. The payload sets
      `merge_queue: true`, and `deleted_branch` is false because the queue
      applies the repository's own head-branch deletion setting.
    Enforced by `pr merge` and the `pr deliver` merge step. `pr deliver` has no
    freeze bypass; a freeze holder merges with `pr merge --allow-merge-freeze`.

19. **PR/MR hold labels.** Independently of repository freezes, direct merge
    and merge-queue enqueue refuse `state::do-not-merge`, `do-not-merge`, and
    `control::hold` with `pr_hold_active` (`DATA 65`). The error names the
    matched label and requests removal by the maintainer or authorized hold
    owner after the release conditions are satisfied. A new head does not lift
    a hold. There is no command-line bypass; freeze overrides do not waive it.
    Both the initial gate and the final pre-mutation gate read current labels
    from the provider with no cache. GitHub reads every REST label page; GitLab
    reads the MR's label array. Failed, missing, or malformed reads refuse the
    operation with `pr_hold_labels_unavailable` (`UNAVAILABLE 69`). Provider
    permissions/protection remain the enforcement layer for changes racing the
    final read and mutation; this gate does not remove already queued entries.
    `[merge].hold_labels` configures additional names; global and repository
    lists are combined with the three compatibility defaults, which cannot be
    removed. The current provider base branch
    configuration is read fresh at both gates and also applies, including
    cross-repository targets; a PR head cannot remove its configured holds.
    Unavailable or malformed base policy reads fail closed. Invalid entries fail with `invalid_hold_labels_config` (`DATA 65`).
    Error details include typed `retryable`, `next_action`, and `recovery` fields;
    callers must branch on those fields and the error code, not message text.

Violations map to `DATA 65` with one of these `data.error.kind` values:

| `error.kind`                               | Triggered by rule     |
| ------------------------------------------ | --------------------- |
| `branch_name_invalid`                      | 1                     |
| `branch_kind_mismatch`                     | 1                     |
| `body_missing_summary`                     | 2                     |
| `body_missing_test_plan`                   | 2                     |
| `title_too_long`                           | 3                     |
| `dirty_worktree`                           | 4                     |
| `head_not_pushed`                          | 5                     |
| `default_branch_protected`                 | 6                     |
| `push_destination_missing`                 | 6                     |
| `push_destination_ambiguous`               | 6                     |
| `push_destination_credentials_unsupported` | 6                     |
| `push_destination_rewrite_ambiguous`       | 6                     |
| `provider_mismatch`                        | 6                     |
| `provider_unsupported`                     | 6 (`USAGE 64`)        |
| `repository_mismatch`                      | 6                     |
| `detached_head`                            | 6                     |
| `default_branch_checkout`                  | 6                     |
| `head_not_checked_out`                     | 6                     |
| `expected_base_mismatch`                   | 6                     |
| `expected_base_missing`                    | 6                     |
| `expected_base_not_ancestor`               | 6                     |
| `direct_commit_count_invalid`              | 6                     |
| `commit_signature_unverified`              | 6                     |
| `reason_file_unreadable`                   | 6                     |
| `reason_invalid`                           | 6                     |
| `object_id_invalid`                        | 6                     |
| `remote_default_lookup_failed`             | 6 (`UNAVAILABLE 69`)  |
| `remote_default_branch_missing`            | 6                     |
| `git_timeout`                              | 6 (`UNAVAILABLE 69`)  |
| `git_output_limit`                         | 6 (`UNAVAILABLE 69`)  |
| `backend_timeout`                          | 6 (`UNAVAILABLE 69`)  |
| `backend_output_limit`                     | 6 (`UNAVAILABLE 69`)  |
| `default_push_rejected`                    | 6 (`RUNTIME 1`)       |
| `default_push_verification_failed`         | 6 (`RUNTIME 1`)       |
| `software_error`                           | 6 (`SOFTWARE 70`)     |
| `draft_merge_refused`                      | 7                     |
| `checks_pending`                           | 8                     |
| `checks_failed`                            | 8 (`RUNTIME 1`)       |
| `checks_not_registered`                    | 8                     |
| `merge_method_unsupported`                 | 9                     |
| `branch_dependencies_unavailable`          | 20 (`UNAVAILABLE 69`) |
| `stacked_branch_auto_delete`               | 20 (`DATA 65`)        |
| `keep_branch_conflict`                     | 10                    |
| `local_path_present`                       | 11                    |
| `agent_attribution_present`                | 17                    |
| `pr_hold_active`                           | 19                    |
| `invalid_hold_labels_config`               | 19                    |
| `pr_hold_labels_unavailable`               | 19 (`UNAVAILABLE 69`) |
| `merge_freeze_active`                      | 18                    |
| `merge_freeze_not_active`                  | 18                    |
| `merge_freeze_ambiguous`                   | 18                    |
| `merge_queue_method_mismatch`              | 18                    |
| `merge_policy_unavailable`                 | 18 (`UNAVAILABLE 69`) |
| `merge_queue_enqueue_rejected`             | 18 (`RUNTIME 1`)      |
| `merge_queue_dequeued`                     | 18 (`RUNTIME 1`)      |
| `merge_queue_checks_failed`                | 18 (`RUNTIME 1`)      |
| `merge_queue_poll_failed`                  | 18 (`UNAVAILABLE 69`) |
| `merge_queue_timeout`                      | 18 (`UNAVAILABLE 69`) |
| `review_changes_requested`                 | 12                    |
| `review_convergence_head_missing`          | 12                    |
| `review_convergence_head_changed`          | 12                    |
| `review_convergence_activity_changed`      | 12                    |
| `review_snapshot_incomplete`               | 12, 16                |
| `invalid_review_convergence_config`        | 12                    |
| `unresolved_review_threads`                | 13                    |
| `unchecked_task_items`                     | 14                    |
| `review_thread_pr_mismatch`                | 15                    |
| `pending_review_not_found`                 | 16                    |
| `pending_review_author_mismatch`           | 16                    |
| `pending_review_not_deletable`             | 16                    |
| `pending_review_head_mismatch`             | 16                    |
| `pending_review_commit_mismatch`           | 16                    |
| `pending_review_body_mismatch`             | 16                    |
| `pending_review_body_too_large`            | 16                    |
| `pending_review_pr_mismatch`               | 16                    |
| `pending_review_inline_comments_present`   | 16                    |
| `pending_review_lease_busy`                | 16 (`UNAVAILABLE 69`) |
| `pending_review_lease_unsafe`              | 16                    |
| `pending_review_reconciliation_failed`     | 16                    |

## Activity output contract

`activity` is read-only and reports recent activity through the same workspace
envelope used by the parity ops. Personal commands remain GitHub-only in v1;
`activity feed` is repo/project-scoped and supports GitHub and GitLab. Local
providers return `provider_unsupported`.

- `activity commits` emits `cli.forge-cli.activity.commits.v1` with
  `data.provider`, `data.host`, `data.user`, `data.since`, `data.limit`,
  `data.item_count`, `data.limited`, and normalized `data.items[]`.
- Commit items include `repo`, `sha`, `url`, `message`, `authored_at`,
  `committed_at`, `author_name`, and `author_email`.
- `activity events` emits `cli.forge-cli.activity.events.v1` with
  `data.provider`, `data.host`, `data.user`, `data.public_only`,
  `data.limit`, `data.item_count`, `data.limited`, and normalized
  `data.items[]`.
- Event items include `id`, `event_type`, `repo`, `actor`, `public`,
  `created_at`, `summary`, and `url`.
- `activity feed` emits `cli.forge-cli.activity.feed.v1` with
  `data.provider`, `data.host`, `data.repo`, `data.since`, `data.limit`,
  `data.item_count`, `data.limited`, and normalized `data.items[]`.
- Feed items include `id`, `external_id`, `provider_event_type`, `kind`,
  `action`, `repo`, `target_kind`, `target_ref`, `target_iid`, `title`, `url`,
  `actor`, `occurred_at`, `summary`, and `details`.
- Feed `kind` and `action` are open vocabulary. Consumers may group common
  values such as `commit` / `committed`, `branch` / `pushed`, and
  `change_request` / `merged`, but unknown provider events stay represented
  instead of being rejected. The provider-native event name is retained in
  `provider_event_type`; provider-specific fields such as refs, commit ranges,
  and GitLab target types live under `details`.
- `activity summary` emits `cli.forge-cli.activity.summary.v1` with
  `data.provider`, `data.host`, `data.user`, `data.since`, `data.limit`,
  `data.total_commit_contributions`, `data.repository_count`,
  `data.limited`, and normalized `data.repositories[]`.
- Summary repository rows include `repo`, `commit_contributions`, and
  `latest_commit_at`.
- `limited=true` means the returned row count reached the requested bound, so
  more provider-side rows may exist.

## Search output contract

`search` is read-only and runs free-text / reverse-reference queries the
structured `issue list` / `pr list` filters cannot express. It delegates to the
provider's search primitives and builds no index. GitHub is the v1 target;
GitLab and Local return a structured `provider_unsupported` error (search is
GitHub-only in v1), never a silent empty result.

Role split (documented in `forge-cli search --help`): `issue list` / `pr list`
filter by structured fields within one repo, `inbox` is the personal cross-repo
work queue, and `search` is full-text and reverse-reference query.

Unqualified queries and `refs-to` use `--repo owner/name` or the detected forge
remote. When no explicit `--repo` is provided, `issues`, `prs`, and `completions` queries with
`org:`, `user:`, or `repo:` qualifiers keep their query scope and omit the
implicit repository filter. Under managed identity they resolve the principal
against its sole distinct profile; ambiguity fails before credential probes.
An explicit `--repo` always keeps normal repository identity rules. Issues, PRs,
and reverse-reference results use the shared normalized
`SearchItem`: `kind` (`issue` | `pr`), `number`, `url`, `title`, `state`,
`repo`, `updated_at`, `labels` (names), and `matched_field` (best-effort; `null` when the provider does not
report which field matched, which the GitHub path never does).

- `search issues <query>` emits `cli.forge-cli.search.issues.v1` and
  `search prs <query>` emits `cli.forge-cli.search.prs.v1`, each with
  `data.provider`, `data.host`, `data.repo`, `data.query`, `data.match_fields`,
  `data.limit`, `data.item_count`, `data.limited`, and normalized `data.items[]`.
  Query-scoped searches have `data.repo=""`; each item retains its repository.
  They run `gh search <issues|prs> <keywords...> [--repo <slug>] --match <fields>
  --limit <n> --json …`. The `--repo` value is always the raw `owner/name`
  slug; the resolved authority is selected separately through the call-local
  `GH_HOST` binding. `--match` defaults to `title,body,comments` and is
  narrowable. A body-only hit the structured `issue list` cannot surface is
  returned; an empty result is a well-formed empty envelope.

  Structured query qualifiers are separate backend keywords, preserving quoted values; empty
  queries and unfinished quotes/escapes fail as `query_invalid`.
- `search completions <query> --kind issue|pr --since <timestamp> --until <timestamp>`
  emits `cli.forge-cli.search.completions.v1` through paginated GraphQL reads.
  It exports canonical closed-issue/merged-PR facts with exact half-open
  completion windows and explicit search/nested coverage. Partial reads retain
  facts and exit zero; consumers inspect coverage before declaring completion.
  See [Completion search v1](search-completions-v1.md) for the stable fields,
  nullable facts, ceilings, continuation fragments, and neutral example.
- `search refs-to <ref>` emits `cli.forge-cli.search.refs-to.v1` with
  `data.provider`, `data.host`, `data.repo`, `data.reference_number`,
  `data.limit`, `data.item_count`, `data.limited`, and normalized
  `data.items[]`. `<ref>` parses as a GitHub URL, `owner/name#number`, or
  `#number` / `number` (repo from context); an unparseable ref is
  `DATA 65` with `error.kind=ref_invalid`. It runs `gh api graphql` over the
  target's `CROSS_REFERENCED_EVENT` timeline items and normalizes the
  referencing issues / PRs into `SearchItem`s.
- All three render the exact backend argv under `--dry-run` (`data.plan`),
  honor `--format json`, and set `limited=true` when the returned row count
  reached the requested bound.

## Inbox output contract

`inbox` is read-only and aggregates personal work discovery across selected
providers:

- `inbox list` emits `cli.forge-cli.inbox.list.v1` with
  `data.providers[]`, `data.limit`, and normalized `data.items[]`.
- `inbox status` emits `cli.forge-cli.inbox.status.v1` with bounded count rows
  grouped by provider, host, kind, and reason. Counts are bounded by the
  effective query limit, not guaranteed global totals.
- `inbox next` emits `cli.forge-cli.inbox.next.v1` with ranked candidates;
  review-requested work ranks ahead of assigned, todo, authored, and broad
  involved work.
- Every item includes `provider`, `host`, `kind`, `reasons`, `repo`, `number`,
  `title`, `url`, `updated_at`, `author`, and `source`.
- `reasons` is a deterministic de-duplicated array. Allowed v1 values are
  `review`, `assigned`, `todo`, `authored`, and `involved`.
- Partial provider success exits `SUCCESS 0`: successful provider items remain
  in `data.items[]`, failed providers are represented in
  `data.providers[].error`, and top-level `warnings[]` carries string warnings under the shared
  workspace envelope contract.
- With `--strict-providers`, any selected provider failure exits `RUNTIME 1`
  with `error.code=provider_failed`; `error.details.providers[]` preserves the
  same provider rows that a non-strict success envelope would have emitted.
- If every selected provider fails, `inbox` exits non-zero through the normal
  `cli_contract` failure envelope with `error.details.providers[]` instead of
  returning an empty successful inbox.
- GitLab VPN readiness failures use `vpn_unavailable` when the configured probe
  failed and `vpn_probe_dependency_missing` when an optional probe dependency,
  such as the `openvpn` CLI, is missing. GitLab backend subprocess timeouts use
  `backend_timeout`.
- Cached fallback items include a `stale` object with `reason`,
  `cached_at_unix`, and `age_seconds`; the provider row remains `ok=false` and
  carries `cache.used=true` so stale data never masquerades as live success.
- GitLab rows come from host-aware `glab api --hostname <host>` calls. The user
  id and username are discovered with `glab api user --hostname <host>` for the
  current invocation and are not persisted. The identity lookup is skipped when
  no remaining query family needs it (for example `--kind assigned --kind
  todo`, or `--item-type issue` paired with reasons that drop the
  identity-dependent families).
- Selected provider adapters and independent query families within a provider
  run concurrently. The output contract — provider order, deduplicated items,
  warnings, and partial/all-failure exit behavior — is deterministic and
  independent of thread completion order. Live wall-clock latency depends on
  `gh`/`glab` and remote API responsiveness and is not asserted in CI.

## CLI output contract conformance

`forge-cli` follows
[`cli-output-contract-v1`](../../../../docs/specs/cli-output-contract-v1.md)
without exception:

- Canonical flag is `--format text|json`. No `--json` boolean alias
  is introduced (new binary, no migration debt).
- Envelope: `{schema_version, ok, data, warnings}`. Snake_case
  throughout. `data` omitted when no payload. `warnings` omitted when
  empty.
- Schema version literals follow `cli.forge-cli.<command>.v1`. Macro
  steps embed their own atom schema versions inside `data.steps[].
  payload.schema_version`.
- Parse / unknown-subcommand errors go through
  `nils_common::cli_contract::emit_parse_error` so `--format json`
  works at the parse-error layer too.
- Sensitive data: backend stderr is captured and tail-trimmed to 2 KiB
  before being placed under `data.error.detail`. Token-shaped strings
  (`gh[ps]_*`, `glpat-*`, `ghr_*`, `gho_*`) are redacted before
  rendering. URLs are kept as-is — they are not secrets and the user
  wants click-through.

## Exit code map

`forge-cli` uses only the six BSD sysexits-aligned constants from
`nils_common::cli_contract::exit`. Discriminators go in
`data.error.kind`, not in numeric exit codes.

| Constant      | Value | `forge-cli` triggers                                                                                                                                                                                                                                                                                         |
| ------------- | ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `SUCCESS`     | `0`   | Op completed; required state achieved.                                                                                                                                                                                                                                                                       |
| `RUNTIME`     | `1`   | Remote semantic failure: required checks failed, merge conflict, draft already ready.                                                                                                                                                                                                                        |
| `USAGE`       | `64`  | Bad CLI syntax, unknown subcommand, unsupported provider.                                                                                                                                                                                                                                                    |
| `DATA`        | `65`  | Lock-down policy violation (any rule above); body parse failure; invalid VPN config.                                                                                                                                                                                                                         |
| `UNAVAILABLE` | `69`  | `gh`/`glab` missing, auth required, remote 5xx/network error, wait-checks expiry when checks were seen but did not finish (an expiry with none ever reported is `DATA 65` / `checks_not_registered`), review-convergence timeout, GitLab VPN probe failure, backend timeout or backend output-limit failure. |
| `SOFTWARE`    | `70`  | Internal invariant violation (backend JSON did not match expected shape).                                                                                                                                                                                                                                    |

Callers (agent-runtime-kit skills, CI scripts) MUST branch on `error.kind`
when finer granularity is needed. Numeric exit codes alone are
intentionally not enough to distinguish "branch name invalid" from
"missing Test plan section" — both are `DATA 65` because both are
"the input did not meet the documented contract". The discriminator
is in the envelope, where consumers parse it deliberately.

## Configuration

Per-repo overrides live in `.forge-cli.toml` at the repo root:

```toml
[merge]
method = "squash"            # squash | merge | rebase
delete_branch = true
hold_labels = []           # additional names; compatibility defaults always apply

[body]
summary_heading = "## Summary"
test_plan_heading = "## Test plan"

[branch]
feature_prefix = "feat/"
bug_prefix = "fix/"

[checks]
timeout = "30m"
interval = "20s"
required_only = true
none = false                          # true: this repo configures no CI; a
none_reason = "<why no CI is needed>" # head with no checks may pass (needs
                                      # a non-empty none_reason)

[inbox]
gitlab_vpn = "off"                    # off | optional | required
gitlab_vpn_check = "tcp:gitlab.example.com:443"
gitlab_vpn_check_timeout = "2s"
gitlab_openvpn_profile = "<local-openvpn-profile>"
provider_timeout = "20s"
strict_providers = false
cache_fallback = false
cache_max_age = "30m"
no_cache = false

[test_first]
require = false                       # opt in to verified evidence for selected kinds
require_kinds = ["feature", "bug"]     # compatibility default; whole-list override

[review_convergence]
require = false                       # compatibility default
quiet_period = "2m"                  # after relevant activity is observed
timeout = "20m"                      # bounds an active convergence wait

[[review_convergence.bots]]
login = "example-review-bot"
mode = "observed"                    # absence never waits or fails
```

### Global config layer

The same schema may live in a user-global file at
`${XDG_CONFIG_HOME:-$HOME/.config}/forge-cli/config.toml`. It supplies defaults
beneath the per-repo `.forge-cli.toml`, so a setting (e.g. `[test_first]
require = true` or `[merge] method = "rebase"`) applies across every repo
without duplicating it into each checkout. A missing global file is not an
error.

The global layer feeds the sections forge-cli actually consumes from
config today — `[merge]`, `[inbox]`, `[test_first]`,
`[review_convergence]`, `[body]`, plus `[checks].none` / `none_reason`.
Repository `[body]` values override global defaults field by field, so either
heading can be configured independently. The other `[checks]` keys and the
`[branch]` keys are parsed (and validated) for forward-compatibility but are
not yet wired into the corresponding command paths at either layer, so values
placed there are accepted but currently inert.

Resolution order for any setting: explicit flag > repo `.forge-cli.toml` >
global `config.toml` > spec default. Inbox env vars sit between explicit flags
and `.forge-cli.toml`. Unknown keys produce a `warnings[]` entry, not an
error — forward-compatibility for v2 fields.

Review convergence has one safety-preserving exception: once global
`[review_convergence].require = true`, repo config may add bots, lengthen the
global/default quiet period, or override the failure timeout, but cannot disable
the gate, remove global bots, or shorten the quiet period. The explicit
`--review-convergence=false` flag remains the intentional per-invocation
override.

`[checks].none` follows the same rule: `pr wait-checks`, `pr merge`, and
`pr deliver` treat `none = true` (with a non-empty `none_reason`) as
`--allow-no-checks`, and a global `none = false` requires checks everywhere —
repo config cannot flip it to `true`. Because the setting loosens a
fail-closed gate, `pr merge` and `pr deliver` read the repo layer from the
`.forge-cli.toml` committed on the PR base branch
(`refs/remotes/<remote>/<base>`) rather than the working tree, which may be the
unmerged PR head; a repository opts out by landing the declaration first.
The declaration also applies only to its own repository: when `--repo` names a
repository other than the checkout's `--remote`, it is ignored.
`pr wait-checks` gates nothing (merge re-checks rule 8) and reads the working
tree. `none = true` without a reason is ignored and the fail-closed default
stays in force. The explicit `--allow-no-checks` flag remains the
per-invocation override.

### `[test_first]` — test-first evidence gate

`require` defaults to `false`; the gate is off unless a repo or the global
config opts in. When it resolves `true`, `pr create` and `pr deliver` (both the
create and adopt paths, and the `--dry-run` preflight) require
`--test-first-evidence <dir>` for kinds selected by `require_kinds`. If omitted,
`require_kinds` defaults to `["feature", "bug"]`, preserving existing behavior.
Set `require_kinds = ["bug"]` to require evidence only for bug PRs while allowing
feature PRs without a record. The list accepts `feature`, `bug`, `docs`, `chore`,
`ci`, `refactor`, and `test`; unselected kinds are exempt. An empty list selects
no kinds. Setting the list alone does not enable the gate: `require` must still
resolve `true`.

The repository list replaces the global list in full, including an explicit
empty list; an omitted list inherits the global setting. A non-array, non-string
member, or unknown kind produces an
`invalid-config-value:test_first.require_kinds:expected_array_of_pr_kinds`
warning and ignores the entire list, retaining the lower-precedence list or
compatibility default. The missing-evidence error names the selected `--kind`
and directs the caller to `--test-first-evidence <dir>`.

The directory must hold a verified v2 `test-first-evidence` record with a testable classification,
an actual changed/added/removed behavior, an affected-test decision, meaningful
failing evidence or an explicit waiver, scoped passing validation, and a
residual-gap declaration. The record must also bind an immutable repository and
pre-edit baseline plus a latest delivery head/tree/diff attestation matching
the explicitly selected delivery ref. Create and adopt compare that attested
head with the provider's immutable PR/MR head OID; deliver re-fetches it after
checks and after ready, and merge uses the same OID as a compare-and-swap
condition. Amend, rebase, or post-check push operations invalidate the latest
delivery attestation until `test-first-evidence bind-delivery` appends a new
attempt; the baseline is never replaced. Record v1 remains readable but cannot
satisfy this gate. Failures surface as `test_first_evidence_required`, `test_first_evidence_v1`,
`test_first_evidence_classification`, `test_first_evidence_incomplete`,
`test_first_evidence_unbound`, `test_first_evidence_subject_mismatch`,
`test_first_evidence_provider_head_unavailable`,
`test_first_evidence_provider_head_mismatch`, or
`test_first_evidence_unreadable` (exit `DATA`).

A failed final attempt from another command remains blocking after a canonical
PASS until explicitly dispositioned with `test-first-evidence resolve-final`.
The refusal names the command and required flags. The original FAIL remains in
the record; create includes final attempt history and disposition in the PR body.
See [failed final validation resolution](../../../agent-workflow-primitives/docs/specs/failed-final-validation-resolution-v1.md)
for reference validation and investigation requirements.

### `[review_convergence]` — observed native review gate

`require` defaults to `false`, `quiet_period` to `2m`, and `timeout` to `20m`.
`quiet_period` is limited to `1h`; `timeout` is limited to `24h`. Invalid
or arithmetically overflowing values are warnings while the feature is
disabled, but fail an enabled merge
with `invalid_review_convergence_config` instead of silently falling back.
Normally `bots` is a whole-list override: a repo list replaces the global list.
When global `require = true`, repo bots are unioned with global bots as part of
the safety exception above. The v1 `observed` mode is intentionally
absence-tolerant. If no configured bot review exists against the current head,
merge continues immediately and `missing_reviewers` remains empty. If relevant
activity exists, each new native review resets the quiet window. The active
wait polls no more often than every 10 seconds. Timeout anywhere in that
end-to-end wait returns `review_convergence_timeout` (`UNAVAILABLE 69`). Timed
backend subprocesses retain at most 8 MiB per output stream; overflow kills the
process group and returns `backend_output_limit` (`UNAVAILABLE 69`).

The positive flag form `--review-convergence` enables the resolved policy for
one `pr merge` or `pr deliver`; `--review-convergence=false` explicitly
disables a repo/global opt-in. Precedence is explicit flag > repo config >
global config > default, subject to the enabled-global safety exception.
Disabling this feature never disables the existing unresolved-thread gate.
`pr close` has no convergence flag and remains unchanged.
For `pr merge --dry-run` and the merge-capable `pr deliver --dry-run`, the
resolved policy is additive output under `data.review_convergence`; the same
GitHub-only provider check runs before the dry/live split.

Environment variables (read once at startup, all optional):

- `FORGE_CLI_GH_BIN` — override `gh` discovery path (testing).
- `FORGE_CLI_GLAB_BIN` — override `glab` discovery path (testing).
- `FORGE_CLI_INBOX_GITLAB_HOST` — default GitLab host for inbox commands.
- `FORGE_CLI_INBOX_GITLAB_VPN`,
  `FORGE_CLI_INBOX_GITLAB_VPN_CHECK`,
  `FORGE_CLI_INBOX_GITLAB_VPN_CHECK_TIMEOUT`, and
  `FORGE_CLI_INBOX_GITLAB_OPENVPN_PROFILE` — local GitLab VPN readiness
  settings. Profile values are input-only and redacted from output.
- `FORGE_CLI_INBOX_PROVIDER_TIMEOUT`,
  `FORGE_CLI_INBOX_STRICT_PROVIDERS`,
  `FORGE_CLI_INBOX_CACHE_FALLBACK`,
  `FORGE_CLI_INBOX_CACHE_MAX_AGE`, `FORGE_CLI_INBOX_NO_CACHE`, and
  `FORGE_CLI_INBOX_CACHE_DIR` — inbox timeout/cache controls.
- `FORGE_CLI_DEFAULT_PROVIDER` — fallback provider when remote URL
  doesn't auto-detect.
- `FORGE_CLI_RATE_LIMIT_GATE` — set to `off`/`0`/`false`/`no` to disable the
  GraphQL rate-limit gate (default enabled).
- `FORGE_CLI_RATE_LIMIT_MIN_REMAINING` — GraphQL budget headroom below which the
  gate waits before a GraphQL-backed call (default `50`).
- `FORGE_CLI_RATE_LIMIT_MAX_WAIT_SECS` — upper bound on the gate's wait for the
  GraphQL budget to recover, per gated attempt (default `120`).
- `FORGE_CLI_RATE_LIMIT_POLL_SECS` — re-probe interval while the GraphQL budget
  is below the threshold (default `15`, minimum `1`).

## GraphQL rate-limit gate

GitHub meters the GraphQL API on a budget separate from REST/core, so the
shared GraphQL budget can be drained by other consumers while REST/core still
has thousands of requests remaining. A subsequent GraphQL-backed call then
fails — historically surfacing as a misleading "not available"/not-found error
that risks a wrong conclusion (sympoies/nils-cli#1051).

Two mechanisms harden this:

- Backend stderr that reports an exhausted rate limit is classified as an
  explicit `backend_rate_limited` error (`UNAVAILABLE 69`) rather than a generic
  `backend_error`, so throttling is never mistaken for a missing resource.
- Every op builds its live backend runner through a single `default_runner()`
  factory rather than a bare process runner, so all GraphQL-backed calls — not
  just the PR-lifecycle ops — preflight the FREE `gh api rate_limit` endpoint
  (which consumes no quota) and wait, bounded by
  `FORGE_CLI_RATE_LIMIT_MAX_WAIT_SECS`, for `resources.graphql.remaining` to
  recover before issuing the call; a `backend_rate_limited` failure then
  triggers one wait-and-retry. Centralizing runner construction keeps the
  budget classifier and the wiring from drifting: a newly-added op cannot ship
  ungated, and a guard test rejects an op that constructs a bare runner. REST
  calls (`gh api repos/…`), the probe itself, and non-GitHub backends are never
  gated. Each probe is bound to the triggering call's resolved authority, and
  cached budget readings are keyed by that canonical authority; a reading or
  invalidation for one GitHub host is never reused for another. The gate is
  best-effort: an unreadable probe never blocks real work.

## Provider detection

Provider resolution binds a provider, canonical authority, and optional
repository path before an operation builds backend argv:

1. `--provider` plus `--host` binds both explicitly. A syntactically valid
   custom authority is accepted unless it is positively classified as the
   other provider.
2. `--host` without `--provider` must classify as GitHub or GitLab.
3. `--provider` without `--host` uses an existing selected remote when that
   remote exposes a valid authority positively classified as the selected
   provider. A conflicting ambient remote is ignored in favor of the provider
   default for repository-independent commands and for repository-scoped
   commands with an explicit `--repo`. A repository-scoped command without
   `--repo` still relies on ambient remote authority, so a mismatched,
   malformed, or unclassified existing remote fails closed. Custom authorities
   always require `--host`.
4. With neither flag, the selected remote authority must classify as GitHub or
   GitLab; unknown authorities return `USAGE 64` with
   `error.kind=provider_unsupported`. Provider-resolution diagnostics never
   echo the raw remote URL or its userinfo.

Classification trusts only controlled provider names/suffixes (`github.com`,
`*.github.com`, `*.ghe.com`, `gitlab.com`, and `*.gitlab.com`); branding-like
prefixes such as `gitlab.attacker.example` are not trusted. `ssh.github.com`
canonicalizes to `github.com`, and `altssh.gitlab.com` to `gitlab.com`.
Non-default HTTPS ports remain part of the selected authority; SSH transport
ports are not treated as API ports.

For a repository-scoped command, explicit `--host` also requires an explicit
`--repo` or a repository derived from a remote with the same canonical
authority; otherwise resolution returns `repo_required` before backend
execution. Provider repository shapes are validated as described under Global
flags. Repository-independent commands retain any repository input for
context but skip its shape validation.

## Migration plan: agent-runtime-kit skills → forge-cli

This is the v1 acceptance target. Every row below MUST be reachable
through `forge-cli` before agent-runtime-kit can adopt the new CLI:

| agent-runtime-kit skill                        | forge-cli op                                            |
| ---------------------------------------------- | ------------------------------------------------------- |
| `create-github-pr` / `create-feature-pr`       | `forge-cli pr create --kind feature`                    |
| `create-bug-pr`                                | `forge-cli pr create --kind bug`                        |
| `close-feature-pr` / `close-github-pr` feature | `forge-cli pr merge` (+ optional `pr ready`)            |
| `close-bug-pr`                                 | `forge-cli pr merge`                                    |
| `deliver-feature-pr` / `deliver-github-pr`     | `forge-cli pr deliver --kind feature`                   |
| `deliver-bug-pr`                               | `forge-cli pr deliver --kind bug`                       |
| `gh-fix-ci`                                    | `forge-cli pr wait-checks` + skill's fix-and-push loop  |
| code-review outcome posting                    | `forge-cli pr review`                                   |
| `example:create-feature-mr` / `-bug-mr`        | `forge-cli pr create` (provider auto-detected gitlab)   |
| `example:close-*-mr` / `deliver-*-mr`          | same as github counterparts                             |
| `issue-lifecycle`                              | `forge-cli issue create|view|edit|comment|close|reopen` |
| `issue-follow-up`                              | `forge-cli issue create` (+ subsequent comments)        |
| program tracker upkeep                         | `forge-cli issue tracker lint|graph|tick`               |

Skills keep their bash shells. The shells:

- Resolve / construct branch names, body content, ticket slugs.
- Call `forge-cli`.
- React to `forge-cli`'s envelope (parse `error.kind`, retry on
  `UNAVAILABLE`, surface `policy` violations to the user verbatim).
- Handle local git operations: `git push`, `git checkout`, branch
  deletion locally, post-merge cleanup.

The shells stop wrapping `gh` / `glab` directly. The `gh pr create …`
and `glab mr create …` invocations are removed.

## Testing strategy

- **Atom unit tests.** Each op has a unit test pair: one against a
  recorded `gh --json …` fixture and one against a recorded `glab …
  -F json` fixture. Fixtures live under
  `crates/forge-cli/tests/fixtures/{github,gitlab}/<op>/`.
- **Exit-code matrix.** Per workspace policy, every binary ships an
  exit-code matrix test covering `success`, `usage`, `data`, and
  `runtime` paths. `forge-cli` extends it to cover `unavailable`
  (forced via `FORGE_CLI_GH_BIN=/bin/false`) and one `software` path
  (mangled fixture).
- **Parity test.** A single test harness drives both backends through
  the same op + the same logical input, then asserts the envelope is
  byte-identical except for `data.provider` and `data.url` host.
- **Dry-run smoke.** Every op supports `--dry-run`; integration tests
  use `--dry-run` to verify the constructed backend argv without
  actually contacting `github.com` or `gitlab.com`.
- **End-to-end is opt-in.** Tests that hit real GitHub / GitLab are
  gated behind `FORGE_CLI_E2E=1` and a designated sandbox repo.
  Default CI does not run them.

## Open questions / v2 candidates

- Releases (`gh release view / edit`) — GitLab requires
  glab + tag flow; could fit a `forge-cli release …` tree later.
- `gh api` passthrough — explicitly out of v1 because it defeats the
  lock-down value. Re-evaluate if a real workflow needs a non-CRUD
  call (e.g. CODEOWNERS, branch protection) and only then.
- Repo creation (`gh repo create`) — out of v1; rarely called from
  skills.
- Issue macros (`issue deliver`, `issue close-when-prs-merged`,
  `issue cross-link`) — tracked separately from the agent-runtime-kit skills.
- Gitea / Forgejo backend — would require a new third backend or a
  Forge-API client. Deliberately deferred until a concrete user
  surfaces.

## Automation read metadata

- `issue list` items include `body` and nullable `updated_at`. GitHub CLI and
  labeled REST list paths both populate them; GitLab maps `description` and
  `updated_at`.
- `issue view` includes nullable `closed_at` and `state_reason`. GitHub state
  reasons are normalized to lowercase (for example `completed` or
  `not_planned`); GitLab has no equivalent state reason and returns `null`.
- GitHub `repo view` includes `default_branch_head_sha`, fetched from the
  default branch commit REST endpoint using the same invocation identity and
  authority. Other providers return `null`. Delivery macros retain their
  existing repository metadata lookup and do not require a commit read.
  GitHub dry-run retains the initial `data.plan` and adds `data.follow_up.plan`
  for the commit read; `<encoded_default_branch>` (and unresolved owner/name
  when necessary) is a dependency placeholder, resolved by the first live read.
- The canonical PR metadata-and-checks read is two calls:
  `forge-cli pr view ID --repo owner/repo --format json`, then
  `forge-cli pr checks ID --repo owner/repo --format json`.
  Checks are a separate snapshot; consumers should use `head_sha` from PR view
  when they need to bind later decisions to a particular head.

## Repository inventory and security reads

These commands are GitHub-only and read-only; other providers return
`provider_unsupported`. Every API call uses the detected authority and the
invocation's managed identity. Provider permission errors remain errors.

- `repo list --org ORGANIZATION [--source] [--no-archived] [--limit N]`
  emits `cli.forge-cli.repo.list.v1`: `provider`, `host`, `org`, `limit`,
  `limited`, and `items`. Each item contains `name`, `full_name`, `url`,
  `private`, `fork`, `archived`, nullable `default_branch`, and nullable
  `updated_at`. Source selection uses GitHub's `type=sources`; archived
  filtering happens before the limit. Pages are fetched until the filtered
  limit is reached or results are exhausted. The default limit is 100.
  Organization inventory has no checkout-repository scope; omit `--repo`.
  Managed principals must select a sole distinct profile, following the
  repository-independent read rules; ambiguity fails before credential probes.
- `security alerts list --repo owner/repo --kind
  dependabot|code-scanning|secret-scanning [--state STATE] [--limit N]`
  emits `cli.forge-cli.security.alerts.list.v1`: `provider`, `host`, `repo`,
  `kind`, `state`, `limit`, `limited`, and `items`. Items contain `number`,
  `state`, `url`, nullable `title`, `severity`, `created_at`, `updated_at`,
  `dismissed_at`, `fixed_at`, and `resolved_at`. The default state is `open`
  and the default limit is 100; pagination honors larger limits. Dependabot
  reads use `--include` to extract the next page's opaque `after` cursor from
  the HTTP `Link` header, and never send the unsupported numbered `page`
  parameter. The cursor is decoded and sent on another request bound to the
  selected repository and authority; pagination stops at the requested limit
  or when no next link remains. Code and secret scanning use numbered pages.
  See [GitHub's Dependabot alert parameters](https://docs.github.com/en/rest/dependabot/alerts#list-dependabot-alerts-for-a-repository).
  Dependabot
  accepts `open`, `dismissed`, `fixed`, and `auto_dismissed`; code scanning
  accepts `open`, `dismissed`, and `fixed`; secret scanning accepts `open`
  and `resolved`. `all` omits the provider state filter. Incompatible states
  fail with `alert_state_invalid`. Alert output selects safe metadata fields
  and never includes scanned secret values or code snippets.
- `security settings view --repo owner/repo` emits
  `cli.forge-cli.security.settings.view.v1`: `provider`, `host`, `repo`, and
  nullable `security_and_analysis`. A missing block means the provider did
  not expose settings to this identity; it does not mean features are disabled.
  GitHub requires appropriate repository or organization permissions to expose
  that block. See the [GitHub repository API](https://docs.github.com/en/rest/repos/repos).
- Security reads use explicit or remote-derived repository identity rules,
  including credential, actor, permission, and repository coverage checks.
- `--dry-run` emits the first page's exact `data.plan` without calling the
  backend. `limited=true` indicates the returned count reached the requested
  bound; it does not prove that more results exist.
