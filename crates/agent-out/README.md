# nils-agent-out

`agent-out` generates and audits canonical `$AGENT_HOME/out/` artifact paths for agent workflows.

The CLI keeps ad hoc project artifacts under:

```text
$AGENT_HOME/out/projects/<project-slug>/<YYYYMMDD-HHMMSS>-<topic>/
```

It does not install hooks or block arbitrary filesystem writes. Hooks and skills can consume this command later.

## Commands

### `project`

Generate a canonical project-scoped run directory path.

```bash
agent-out project --topic "api smoke" --repo . --mkdir
agent-out project --topic "api smoke" --repo-slug sympoies/nils-cli --format json
agent-out project --topic "api smoke" --format env
```

Options:

- `--topic <TOPIC>`: required run label; sanitized for path safety.
- `--repo <PATH>`: repository used for slug discovery; defaults to the current directory.
- `--repo-slug <OWNER/REPO>`: explicit slug source; `owner/repo` becomes `owner__repo`.
- `--agent-home <PATH>`: agent home root; defaults to `AGENT_HOME`.
- `--mkdir`: create the generated directory.
- `--format path|json|env`: output mode; default is `path`.

Slug precedence:

1. `--repo-slug owner/repo` becomes `owner__repo`.
2. Git `origin` remote under `--repo` or the current directory becomes `owner__repo`.
3. Local fallback becomes `local__<basename>-<short-hash>`.

### `path-for`

Compatibility allocator for rendered `state_out(...)` helper calls that emit
`agent-out path-for --domain ...`. It returns the same canonical project-scoped
run directory shape as `project` while accepting the older domain/topic flags:

```bash
agent-out path-for --domain projects --topic "daily brief" --mkdir
agent-out path-for --domain tools --repo sympoies/nils-cli --format json
agent-out path-for --domain projects --topic "project retro" --format env
```

Options:

- `--domain <DOMAIN>`: required compatibility domain; sanitized for path safety.
- `--topic <TOPIC>`: optional artifact topic. For `--domain projects`, the
  topic is used directly. For other domains, the topic becomes
  `<domain>-<topic>`.
- `--repo <PATH_OR_OWNER/REPO>`: existing paths are used for slug discovery;
  owner/repo-looking values are treated as explicit slugs.
- `--repo-slug <OWNER/REPO>`: explicit slug source; overrides slug discovery.
- `--agent-home <PATH>`: agent home root; defaults to `AGENT_HOME`.
- `--mkdir`: create the generated directory.
- `--format path|json|env`: output mode; default is `path`.

### `audit`

Scan top-level entries under `$AGENT_HOME/out/` and separate canonical or allowlisted roots from noncanonical ad hoc entries.

```bash
agent-out audit
agent-out audit --strict
agent-out audit --format json
```

The MVP allowlist covers the canonical `projects/` root, current home-scope policy roots,
and explicit tool/workflow roots already documented in nils-cli or agent-runtime-kit:

- `projects`
- `agent-browser`
- `api-test-runner`
- `delegate-parallel`
- `image-processing`
- `macos-agent-trace`
- `plan-issue-delivery`
- `plan-issue-sprint-pr`
- `playwright`
- `screen-record`
- `screenshot`
- `semgrep`
- `tests`
- `workspace-shared-audit`
- `workspace-test-cleanup`

New top-level roots should be added deliberately when they become a stable tool contract.

### `cleanup`

Build and apply reviewed cleanup plans for `$AGENT_HOME/out/`.

`cleanup plan` is dry-run only:

```bash
agent-out cleanup plan --format json
agent-out cleanup plan --include-projects --format json > cleanup-plan.json
```

Plan classification is conservative:

- `$AGENT_HOME/out` must be a real directory; cleanup refuses symlinked out
  roots instead of treating the symlink target as the deletion boundary.
- `nils-versions` is a `cache` delete candidate because it can be recreated
  from release assets.
- top-level noncanonical entries without retained evidence markers are reported
  as `needs-policy`; they are not deleted by default because documented
  workflows may place reviewed reports directly under `$AGENT_HOME/out/`.
- any directory containing `skill-usage.record.json` or
  `test-first-evidence.json` is preserved; use `evidence migrate` and
  `evidence prune-source --archived-only` for `skill-usage` source cleanup.
- canonical and allowlisted preserved roots are listed with shallow metadata
  unless children are explicitly requested; their marker booleans are not a
  recursive assertion about all descendants.
- `projects/<repo>/<run>` entries are included only with `--include-projects`;
  runs without evidence markers are reported as `needs-policy`, not deleted,
  unless the owner opts into a retention window (below).
- an entry that cannot be fully read (for example a subtree owned by another
  account) no longer fails the whole plan. It becomes a `preserve` row with
  category `unreadable` and a one-line `diagnostic` (`code`, `path`,
  `message`), is counted in `summary.unreadable`, and is never deleted. Failing
  to list `$AGENT_HOME/out` or `projects/` itself still fails the plan.

#### Project-run retention

```bash
agent-out cleanup plan --include-projects --project-retention-days 30 --format json
```

`--project-retention-days <DAYS>` (requires `--include-projects`, `DAYS >= 1`)
records `project_retention_days` and `project_retention_cutoff_unix` in the plan
and its digest. A project run becomes a `delete` candidate only when all of
these hold:

- it is a real directory with no evidence markers;
- its name starts with an allocation run id (`YYYYMMDD-HHMMSS-…` or
  `YYYYMMDD-…`) earlier than the cutoff. The id is local time read as UTC, which
  on hosts east of UTC only delays deletion;
- nothing in the tree was modified at or after the cutoff;
- every directory in the tree has the same owner as `$AGENT_HOME/out`, so the
  delete cannot stop half way on another account's files.

Other runs stay `needs-policy` with a reason naming the failed condition. A
delete row carries a `tree_identity`: a versioned metadata digest of relative
path, type, size, mtime, ctime, device, and inode for every entry. Hashing file
contents at plan and again at apply is impractical for multi-gigabyte runs, and
any rewrite, rename, or replacement changes this identity. Without the flag,
plans are byte-identical to earlier releases.

`cleanup apply` requires a reviewed plan file and exact digest confirmation:

```bash
agent-out cleanup apply \
  --plan-file cleanup-plan.json \
  --confirm-digest sha256:<digest> \
  --agent-home "$AGENT_HOME" \
  --format json
```

Apply deletes only reviewed `cache` delete items from the plan. It rejects
digest mismatches, requires the resolved agent home to match the plan, rejects
parent-directory or out-of-root delete paths, validates every delete candidate
before removing any path, rejects duplicate delete paths, re-checks evidence
markers immediately before deletion, and skips stale items whose size,
modification time, or content digest changed after planning. Cache delete items
must include `content_digest`. Older v1 plans that contain
`top-level-noncanonical` delete items or cache delete items without
`content_digest` must be regenerated; apply now fails those closed instead of
deleting them. If `--agent-home` is omitted, `AGENT_HOME` is still required so
the plan has a live runtime-root boundary.

Project-run delete rows are accepted only as exact `projects/<repo>/<run>`
paths in a plan whose retention policy is consistent and whose cutoff is no
later than `now - DAYS` (`cleanup-retention-policy-invalid` otherwise). Before
each deletion apply re-reads the run: a read failure, a new evidence marker, a
changed `tree_identity`, or a run that no longer meets the policy becomes a
`skipped` entry. A project-run delete that fails part way is recorded as
`failed` (`summary.failed`) and the remaining rows continue; other categories
still abort on a delete failure.

### `completion`

Print generated shell completions:

```bash
agent-out completion zsh > completions/zsh/_agent-out
agent-out completion bash > completions/bash/agent-out
```

## Output Contracts

Human-readable mode is the default. Primary command output goes to stdout; errors go to stderr.

JSON output is opt-in and uses versioned envelopes:

- `cli.agent-out.project.v1`
- `cli.agent-out.path-for.v1`
- `cli.agent-out.audit.v1`
- `cli.agent-out.cleanup.plan.v1`
- `cli.agent-out.cleanup.apply.v1`

Example:

```json
{
  "schema_version": "cli.agent-out.project.v1",
  "command": "agent-out project",
  "ok": true,
  "result": {
    "path": "/home/user/.agents/out/projects/sympoies__nils-cli/20260511-121314-api-smoke",
    "agent_home": "/home/user/.agents",
    "out_root": "/home/user/.agents/out",
    "repo": "/work/nils-cli",
    "project_slug": "sympoies__nils-cli",
    "topic": "api-smoke",
    "run_id": "20260511-121314-api-smoke",
    "created": false
  }
}
```

## Exit Codes

- `0`: success
- `1`: runtime failure, or audit violations when `audit --strict` is used
- `64`: usage/configuration error, including missing `AGENT_HOME`
- `65`: invalid cleanup plan data, including digest mismatches

## Docs

- [Docs index](docs/README.md)
