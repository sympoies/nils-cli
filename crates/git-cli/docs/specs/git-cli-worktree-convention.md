# git-cli Worktree Convention

## Ownership

This is a crate-local specification for `git-cli worktree`.

## Path Convention

`git-cli worktree add <slug>` creates worktrees under:

```text
$AGENT_HOME/worktrees/<repo-key>/<branch-slug>
```

- `<repo-key>` is `<repo-basename>-<short-hash>`, where the short hash is a
  stable hash of the absolute repository root. This prevents collisions between
  repositories with the same basename.
- `<branch-slug>` is a filesystem-safe, lowercase slug derived from the user
  argument.
- The new branch is always `feat/<branch-slug>`.

If `AGENT_HOME` is unset, the CLI falls back to
`${XDG_STATE_HOME:-$HOME/.local/state}/agent-runtime-kit`.

## Commands

- `git-cli worktree add <slug> [--from <ref>] [--kind <kind>] [--format text|json]`
- `git-cli worktree list [--format text|json]`
- `git-cli worktree remove <slug-or-path> [--safe] [--format text|json]`
- `git-cli worktree prune [--format text|json]`
- `git-cli worktree go <slug-or-branch-or-path> [--shell] [--format text|json]`

`--kind` selects the branch prefix (`feature`->`feat/`, `bug`->`fix/`,
`chore`->`chore/`, `docs`->`docs/`, `ci`->`ci/`, `refactor`->`refactor/`,
`test`->`test/`);
default `feature`.

`remove` refuses primary, current, and unmanaged worktrees. It never forces
removal. Use `git-cli worktree remove <slug-or-path> --safe --format json` from
outside the target; the explicit flag makes older binaries fail closed before
their forced removal. Current binaries always apply the same safety checks.

The cleanup transaction holds a shared checkout lifecycle fence, the runtime
checkout lease lock, and the agent-session registry lock through removal, in
that acquisition order. Matching `agent-session` launch and resume paths hold
the lifecycle fence before publishing startup state or entering tmux until
registration or failure rollback completes. Install matching released launch
and removal implementations with the same `AGENT_SESSION_STATE_DIR` (or default
state root); older launchers do not participate in this protocol.

It requires clean stable checkout/admin identity,
no Git operation or active checkout lease (including the requester), no live
session cwd/binding or nonterminal operation, and a complete `lsof` inventory
with no process cwd or open file under the target. Missing tools, warnings,
malformed ownership state, lock contention, and unavailable proof retain the
target. Coordination mode does not waive these checks. Runtime and session state
roots use their existing environment configuration.

HEAD must be present in the current `origin` default branch, or match the exact
head of a provider-confirmed merged PR/MR targeting that default branch.
Remote/default proof is fetched rather than inferred from cached refs; provider
proof uses `forge-cli`. This also covers squash/rebase merges and deleted remote
feature branches. Unpushed or unmerged HEADs are retained. Removal leaves the
local branch intact and prunes stale worktree metadata. The JSON receipt includes
`removed_branch`, `removed_head`, and `delivery_proof`: its `basis` is
`remote-default-ancestry` or `provider-exact-head-merge`, with `default_branch`,
`default_head`, and the provider `pr_number` for the latter.

`go` resolves a single worktree (in priority order: exact branch name, explicit
worktree path, managed slug, then worktree directory basename) and prints its
path so the caller can `cd` into it. `--shell` prints an evaluable
`cd -- <path>` command instead of the bare path, mirroring `utils root --shell`;
the committed `gxwcd` shell helper wraps it and adds worktree-name completion.

## Upstream Tracking

`add` creates the branch with `--no-track`. The default base ref is the cached
remote default branch (`origin/main`), and Git's `branch.autoSetupMerge` default
would otherwise record *that* as the new branch's upstream — leaving
`branch.<new>.merge = refs/heads/main`. Every consumer that reads `@{upstream}`
to find the branch head would then resolve the default branch instead, which is
how `forge-cli pr deliver` came to report an already-pushed head as unpushed.

A managed worktree branch is unpublished, so it has no upstream. The upstream is
established at publish time by `git-cli push`, which sets it to the branch's own
ref. See [git-cli remote surfaces](git-cli-remote-surfaces.md).

## Primary Worktree Resolution

The managed layout — `<repo-key>`, the managed/external classification, and
slug-based `add`/`remove`/`go` resolution — is anchored to the repository's
*primary* worktree, resolved as the first entry of `git worktree list`. This is
independent of the worktree the command is invoked from, so `git-cli worktree`
behaves identically from the primary checkout or from inside any linked
worktree. (`git rev-parse --show-toplevel` would otherwise return the current
linked worktree and make the managed namespace diverge.)

## JSON Contract

Every command accepts `--format text|json`. JSON output uses the shared
workspace envelope:

- `cli.git-cli.worktree.add.v1`
- `cli.git-cli.worktree.list.v1`
- `cli.git-cli.worktree.remove.v1`
- `cli.git-cli.worktree.prune.v1`
- `cli.git-cli.worktree.go.v1`

Error responses use stable `error.code` values such as `branch-exists`,
`worktree-path-exists`, `worktree-not-found`, `refuse-primary-worktree`, and
`removal-dirty`, `removal-unmanaged`, `removal-process-active`,
`removal-session-active`, `removal-head-undelivered`, `removal-target-changed`,
`removal-lifecycle-busy`, `removal-lease-active-or-unavailable`, and
`removal-proof-unavailable`.

`git-cli worktree` and `git-cli branch cleanup --remove-worktrees` share the
worktree listing parser and managed path convention. Branch batch cleanup routes linked candidates through the same fence and retains
their branch when proof fails. Agent shell hooks require the explicit sole
`worktree remove --safe` command before separate branch cleanup, because older
batch-cleanup binaries cannot attest this contract.
