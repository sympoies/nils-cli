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
- `git-cli worktree remove <slug-or-path> [--safe] [--acknowledge-backup-omissions] [--format text|json]`
- `git-cli worktree restore <backup-ref> [--path <dir>] [--format text|json]`
- `git-cli worktree backup list [--format text|json]`
- `git-cli worktree backup prune [--older-than <dur>] [--dry-run] [--format text|json]`
- `git-cli worktree prune [--format text|json]`
- `git-cli worktree go <slug-or-branch-or-path> [--shell] [--format text|json]`

`--kind` selects the branch prefix (`feature`->`feat/`, `bug`->`fix/`,
`chore`->`chore/`, `docs`->`docs/`, `ci`->`ci/`, `refactor`->`refactor/`,
`test`->`test/`);
default `feature`.

`remove` refuses primary, unmanaged, and unknown worktrees. `--safe` remains
accepted as a compatibility no-op. Removal prevents known collisions; it does
not require delivery, network, provider access, or complete process visibility.
It runs `git worktree remove --force` and prunes metadata from the primary
checkout. Removing the caller's current worktree leaves its parent shell in a
deleted directory; change to an existing directory afterwards.

Positive evidence retains the target with exit 65 and structured details:

| Code | Evidence |
| --- | --- |
| `removal-session-active` | Foreign running session cwd inside the target, or a foreign active claim/nonterminal operation bound to it |
| `removal-lease-active` | Foreign unexpired checkout lease |
| `removal-process-active` | Readable process cwd, open descriptor, or mapping inside the target; details identify PID and command name |
| `removal-git-busy` | Target Git `index.lock`, `HEAD.lock`, or ref lock modified within the last hour |
| `removal-lifecycle-busy` | Another launch/removal holds the target lifecycle lock |

The caller's `AGENT_SESSION_ID` excludes its own session and ownership records;
its matching hashed checkout lease is released after preservation succeeds.
The CLI process and its ancestors are excluded from the process inventory.
Linux scans readable `/proc` cwd, fd, and maps surfaces independently; a denied
surface cannot hide a readable holder on another surface. Other platforms use
`lsof` stdout. Unreadable processes (including hidepid/nondumpable processes),
`lsof` stderr, unavailable or unknown registry data, and stale Git locks produce
receipt warnings. This is a best-effort collision inventory: opaque holders
cannot be proven absent and may be missed. No elevated privilege is required.

The transaction retains the shared lifecycle lock through snapshot and deletion.
Launchers and removers must use the same physical checkout namespace:
`AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME`, then
`AGENT_RUNTIME_STATE_HOME/checkout-leases`, then
`XDG_STATE_HOME/agent-runtime-kit/checkout-leases`, then the corresponding
`HOME/.local/state` default. Session inventory uses `AGENT_SESSION_STATE_DIR`
or the ordinary agent-session state default. Inventory binding mismatch is a
warning; it does not rebind the persistent inventory. Unknown or unavailable
lease/registry state also warns. Existing readable collision evidence still
blocks in every coordination mode. Stable lock files are never unlinked to
bypass exclusion. Install matching lifecycle-aware launchers and removers.

### Automatic preservation and restore

Dirty tracked files, non-ignored untracked files, Git operation state, or HEAD
unreachable from any local branch, tag, or cached remote ref trigger a snapshot.
A temporary index builds a working-tree commit with parent HEAD; the real index
is unchanged. Its JSON commit message records the original path, branch, HEAD,
operation markers, UTC time, and any acknowledged omissions. The commit is
anchored in `refs/worktree-backup/<slug>/<UTC-timestamp>` in the common repository.
The local branch stays intact. Ignored untracked content is not preserved.
Snapshots describe the working tree; staging distinctions and Git operation
control files are not replayed. Nested repository/submodule content that cannot
be captured retains the target with `removal-backup-failed`.

Configuration is ordinary repository Git configuration:

| Key | Default | Behavior |
| --- | --- | --- |
| `worktree.backupMaxBytes` | `52428800` (50 MiB) | Total non-ignored regular-file/symlink bytes in a snapshot; largest paths are omitted first until under the cap |
| `worktree.backupRetention` | `30d` | Expiry age (`30d`, `1h`, `60s`, etc.); `keep` disables expiry |

An over-cap backup refuses with `removal-backup-acknowledgment-required` and
lists paths/sizes before any deletion. Review the omissions and explicitly pass
`--acknowledge-backup-omissions` to permit their loss. An omitted tracked edit
restores the file's HEAD content; an omitted untracked file is absent on restore.
Snapshot failures retain the target as `removal-backup-failed`, with bounded
reason details that do not copy arbitrary subprocess diagnostics.
Invalid cap configuration uses the default;
invalid retention keeps backups with a warning. No expiry runs inside removal.

The JSON removal receipt includes `removed_path`, `removed_branch`,
`removed_head`, `pruned`, `backup_ref` (or null), `backup_reasons`,
`backup_retention`, `backup_bytes`, `backup_max_bytes`, `backup_omitted_bytes`,
`backup_omissions` (path/bytes), `warnings`, and informational `delivered`.
`delivered` only tests ancestry against the cached `refs/remotes/origin/HEAD`
default branch. It is false when that local default is unavailable; no remote
probe, fetch, or provider call occurs. The retained `delivery_proof` field is null
when undelivered; otherwise its `basis` is `cached-origin-default-ancestry`,
with `default_branch` and `default_head`. This is local informational evidence.

List backups and their sizes with `git-cli worktree backup list --format json`
(or list raw refs with `git for-each-ref refs/worktree-backup`). Restore by full ref:

```sh
git-cli worktree restore refs/worktree-backup/<slug>/<UTC-timestamp> --format json
```

`restore` recreates the recorded path (or `--path`) on the recorded branch if it
still points at the snapshot parent, otherwise detached at that parent. It
replays the snapshot tree with the real index at the parent: saved modifications
are uncommitted and new files are untracked. Existing paths are refused; a branch
already checked out elsewhere must be freed before restoring on that branch.
The backup ref is retained. Omitted/ignored files are not recovered.

Preview expiry with `git-cli worktree backup prune --dry-run --format json`.
The owning command `git-cli worktree backup prune --format json` expires
eligible refs according to `--older-than <dur>` or `worktree.backupRetention` and logs pending/result
records to the common Git directory's `logs/worktree-backup-expiry.jsonl` before
and after compare-and-delete. Unknown backup metadata is retained with a warning;
a missing/busy log retains candidates. Ref deletion is not a Git object prune.
Schedule this command through normal repository maintenance if automatic expiry
is desired; it is not a background daemon. Retention `keep` preserves all refs.

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
- `cli.git-cli.worktree.restore.v1`
- `cli.git-cli.worktree.backup.list.v1`
- `cli.git-cli.worktree.backup.prune.v1`
- `cli.git-cli.worktree.prune.v1`
- `cli.git-cli.worktree.go.v1`

Error responses use stable `error.code` values such as `branch-exists`,
`worktree-path-exists`, `worktree-not-found`, `refuse-primary-worktree`,
`removal-unmanaged`, the collision codes above, `removal-target-changed`,
`removal-backup-failed`, and `removal-backup-acknowledgment-required`.

`git-cli worktree` and `git-cli branch cleanup --remove-worktrees` share the
managed removal transaction, including backup and collision checks. Batch
cleanup leaves the branch when removal fails and prints backup receipts on
success; its separately confirmed branch deletion still follows branch cleanup
policy. Agent hooks may require the sole explicit `worktree remove --safe`
command on mixed installations; use separate branch cleanup after removal.
