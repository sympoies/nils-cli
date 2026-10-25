# forge-cli: Publication and administration

Part of the [forge-cli v1 contract](forge-cli-spec-v1.md). Shared provider
parity, naming, [lock-down rules](forge-cli-spec-v1.md#lock-down-policy),
[exit codes](forge-cli-spec-v1.md#exit-code-map), and
[configuration](forge-cli-spec-v1.md#configuration) remain in the main spec.
The [operation catalog](forge-cli-ops-v1.yaml) owns backend mappings,
validation rules, and output schema versions.

## Operations

### `release create` / `release upload`

GitHub-only repository writes; both return `provider`, `repository`, `tag`,
`url` (nullable), and the requested asset count in `assets` under
`cli.forge-cli.release.create.v1` or `cli.forge-cli.release.upload.v1`.

- `release create <tag> --title <title> --notes-file <path|-> [files...]`
  accepts `--target <branch|sha>`, `--verify-tag`, `--draft`, `--prerelease`,
  and `--latest <true|false>`. Explicit title and notes prevent editor prompts.
  Without `--verify-tag`, GitHub may create a missing tag from `--target` or
  the default branch, matching the backend's release-create behavior.
- `release upload <tag> <files...> [--clobber]` uploads readable local files
  to an existing release. `--clobber` explicitly replaces matching assets.
- Asset arguments are literal files, canonicalized before the backend call;
  shell glob expansion is the caller's responsibility. Title and notes use
  the shared provider payload guards. Validated notes are copied into a scoped
  temporary file for `--notes-file` transport, including stdin input, so large
  notes do not consume process argument space or expand dry-run plans.
- Success records backend completion; it does not claim independent readback
  of every asset. A create URL comes from backend stdout; upload has no URL
  when the backend supplies none.

### `workflow dispatch`

`workflow dispatch <workflow-id|file.yml|file.yaml> --ref <branch|tag>
[--input KEY=VALUE ...]` emits `cli.forge-cli.workflow.dispatch.v1` with
`provider`, `repository`, `workflow`, `git_ref`, and `dispatched: true` after
backend acceptance. The ref is required, input keys must be unique, and values
remain literal strings (including leading `@` and embedded `=`). `-f` aliases
`--input`. `--inputs-file <path|->` accepts a JSON object of string values
instead of inline inputs, enabling large UTF-8 values without argv limits;
duplicate keys and non-string values are rejected. This command neither waits for a run nor infers a run ID from a
concurrent workflow list.

### `comment edit` / `comment delete`

`comment edit <database-id> (--body <text>|--body-file <path|->)
[--kind issue|review] [--expected-updated-at <timestamp>]` emits
`cli.forge-cli.comment.edit.v1` with `provider`, `repository`, `id`, `kind`,
`url`, `body`, `updated_at`, and `deleted: false`. The provider response must
match the requested id and body; `updated_at` is the timestamp returned by the
successful PATCH. Empty or guarded payloads fail before writing.

Before PATCH, the normal runner GETs `user` to identify the authenticated actor,
then GETs the comment endpoint to read its id, author login, and `updated_at`.
Both reads retain the edit's `api_write` identity scope. Author logins are
compared without ASCII case sensitivity; a different author is refused with
`DATA 65` and `comment_author_mismatch`, even without a freshness option. When
`--expected-updated-at` is supplied, its exact string must equal the current
provider `updated_at`; otherwise `DATA 65` and `comment_stale` refuse the edit
before PATCH. Read the comment again before retrying a stale edit. A mismatched
comment id yields `comment_response_mismatch`; an empty actor/author login or
empty snapshot timestamp yields `comment_snapshot_invalid`, both `DATA 65`.
Malformed or incomplete preflight JSON yields `SOFTWARE 70` / `software_error`;
backend failures remain errors and prevent the PATCH.

The freshness guard is read-then-write, not provider atomic compare-and-swap
(CAS). Another edit after the preflight read can still be overwritten. Edits
sharing the same `updated_at` string are also indistinguishable to this guard.
Omitting the option skips only the timestamp comparison; ownership is still
checked. Offline `--dry-run` validates local inputs and emits the PATCH plan
without making either preflight GET or proving ownership/freshness.

`comment delete <database-id> [--kind issue|review]` emits
`cli.forge-cli.comment.delete.v1` with `provider`, `repository`, `id`, `kind`,
and `deleted: true` after a successful DELETE. IDs must be positive.

`issue` (default) identifies issue and PR timeline comments, using
`repos/{repository}/issues/comments/{id}`. `review` identifies diff review
comments, using `repos/{repository}/pulls/comments/{id}`. These are database
IDs, not issue/PR numbers or GraphQL node IDs; provider permissions still govern
which actor can edit/delete a comment.

All five writes resolve their repository from `--repo` or the selected remote,
carry an explicit repository locator or REST endpoint and authority, and pass
through the normal runner's `prepare_api` credential/actor binding. An active
identity requires `api_write`; missing credentials, denied operations, and
unsupported providers fail closed without using ambient authentication.
Workflow dispatch POSTs to the repository's workflow dispatch endpoint; comment
PATCH and workflow POST bodies use scoped JSON files with `gh api --input`.
The payload files remain alive through backend completion and are then removed;
large UTF-8 inputs stay out of argv and dry-run plans.
GitLab and Local are unsupported. `--dry-run` validates local inputs and emits
the normal backend plan without credential probing or provider mutation.
Operation-effect metadata names each command and declares `mutation` /
`network_write`. These commands do not authorize a release or comment deletion;
the caller must already have authority for the requested operation.

### `label audit` / `label ensure`

- Input: `--catalog <path>` pointing at a caller-owned YAML/JSON catalog with
  `groups[]` and `labels[]`. Labels declare `name`, `group`, `color`,
  `description`, and `applies_to`.
- `label audit` emits `cli.forge-cli.label.audit.v1` with
  `data = { provider, status, missing, drift, unknown_shared }`.
- `label ensure` emits `cli.forge-cli.label.ensure.v1` with
  `data.actions[]` create/update plans. Missing labels are created; existing
  label drift is updated only with `--update-existing`.
- Provider behavior:
  - GitHub: `gh label list/create/edit`.
  - GitLab: `glab label list/create/edit`.
- Non-goals: deletion, rename-by-default, and moving catalog ownership into
  `nils-cli`.

### `issue tracker lint` / `graph` / `tick`

These atoms maintain a program tracker issue: an issue whose body holds a
`## Phase table` of checkbox rows and a `## Dependency graph` section with one
derived `mermaid` block. The row grammar, the graph derivation, and the finding
codes are owned by the `agent-runtime-kit` repository
(`tracker-row-grammar.md`). `src/tracker/` implements that grammar as a pure
module, and its conformance corpus is vendored unchanged under
`tests/fixtures/tracker-row-grammar/`.

- A finding is `{ code, line, ids, message }`. `line` is the 1-based line in the
  tracker body, or `null` for a finding about the whole tracker. Grammar codes:
  `malformed-row`, `duplicate-id`, `unknown-dependency`, `self-dependency`,
  `cycle`, and `stale-graph`, which is reported only when the table has no
  other grammar finding. Provider codes: `missing-tracking-label`,
  `state-mismatch`, and `unreadable-ref`. Limit codes: `too-many-rows` and
  `too-many-refs`, each a single whole-tracker finding (`line = null`,
  `ids = []`).
- **Limits.** A tracker body is author-controlled text, so the work it can ask
  for is bounded. A phase table with more than 500 row lines, valid or
  malformed, is not analysed: `lint` reports `too-many-rows` as its only table
  finding (no row, graph, or state finding), `graph` refuses with it, and
  `tick` refuses with `tracker_too_many_rows` (`DATA 65`). 500 rows is the
  bound the downstream board applies. `--check-state` reads at most 200
  distinct issues: above that it reads none of them and reports the single
  finding `too-many-refs`, whose message states the count and the limit.
- `issue tracker lint <id>` emits `cli.forge-cli.issue.tracker.lint.v1` with
  `data = { source, provider, number, url, row_count, state_checked, findings }`.
  It reports the grammar findings, plus `missing-tracking-label` when the issue
  does not carry `workflow::tracking`.
- `--check-state` also reads each issue the rows reference, once, through the
  `issue view` call: `#N` in the tracker's repository and `owner/repo#N` in
  that repository. It reports `state-mismatch` once per row whose checkbox
  disagrees with its issue: the row is ticked but the issue is open, or the row
  is open but the issue is closed or merged. An issue named by several rows is
  delivered in steps and judged as a whole, so an open issue disagrees with its
  rows only when all of them are ticked. `#N` and `owner/repo#N` for the
  tracker's own repository are one issue.
- A referenced issue that does not exist or is not accessible, which the
  backend reports as `backend_error`, is an `unreadable-ref` finding on its
  row. So is `owner/repo#N` for another repository under a local store, which
  holds one repository. Every other read failure is returned as the command's
  error with its own exit code, not as a finding: a throttled or
  unauthenticated provider is `UNAVAILABLE 69`, and a reply that cannot be
  interpreted is `SOFTWARE 70`. A failure the backend cannot classify, such as
  a network error in the middle of a run, is also a plain `backend_error` and
  is reported as `unreadable-ref`; the finding's message carries that kind.
- Any finding exits `DATA 65` with `ok = false` and `error.code =
  tracker_findings`, and the envelope still carries `data`, so consumers read
  `data.findings[]` on both outcomes.
- `issue tracker graph <id>` emits `cli.forge-cli.issue.tracker.graph.v1` with
  `data = { source, provider, number, url, graph, current, change, changed,
  written, dry_run, findings, actions? }`. `graph` is the generated Mermaid
  source without its fence, and text mode prints the fenced block. A table with
  row findings has no generated graph: the command refuses with those findings
  in the same `tracker_findings` failure envelope and `graph = null`.
- `graph --write` changes only the block inside the `## Dependency graph`
  section. A section without a block gets the block right after its heading; a
  body without the section gets the section right after the phase table
  section. No other byte of the body changes, inserted lines follow the line
  ending in use where they land, and nothing is written when the block is
  already current.
- `graph --write` needs a `## Phase table` section. A body without one is not a
  tracker, so the write is refused with `tracker_no_phase_table` (`DATA 65`)
  and no edit call, for an issue and for a `--body-file` draft alike: a
  mistyped issue id must not append a graph section to an unrelated issue. A
  body that has the section but no rows still gets the zero-row block
  (`graph LR`). `lint` and a read-only `graph` keep the grammar's behaviour for
  a body without the section: no rows and the zero-row graph.
- `current` is the staleness signal: whether the body's block already holds the
  generated lines. `change` and `changed` describe the `--write` edit only:
  `change` is `replaced-block`, `inserted-block`, or `inserted-section`, and
  `changed = true`, when `--write` edits the body. Without `--write`, and when
  the block is already current, they are `none` and `false`, whatever `current`
  says. `written` is `true` only when the edit was applied.
- `issue tracker show <ref>` emits `cli.forge-cli.issue.tracker.show.v1` with
  `data = { source, provider, number, url, repo, title, state, labels,
  row_count, rows, findings }`. `<ref>` is `N`, `#N`, or `owner/repo#N`; a
  repository in the ref replaces `--repo`. It makes one `issue view` call and
  no write. Each `rows[]` entry holds `id`, `title`, `reference`, `done`,
  `phase`, `after`, `notes`, and `line`, in table order: `reference` is `owner/repo#N` (an
  own-repository `#N` is qualified with `repo` when it is known) or `null` for a
  gate, `phase` is the third-level heading above the row or `null`, and `line` is the
  1-based body line. Row findings do not fail the command: `rows` lists every
  valid row and `findings[]` reports `malformed-row`, `duplicate-id`,
  `unknown-dependency`, `self-dependency`, and `cycle`, so a board shows the
  lanes of an imperfect tracker. `stale-graph` and the label check are `lint`'s
  and are not reported. A table over 500 rows is not analysed: `rows` is empty
  and `too-many-rows` is its only finding. A body without a `## Phase table`
  section has zero rows. A row whose ref does not end the row text before its
  notes (`title: #12; depends on T1 (PR #13)`) is a gate by the grammar and has
  `reference = null`.
- `issue tracker tick <id> --item <item-id>` emits
  `cli.forge-cli.issue.tracker.tick.v1` with `data = { provider, number, url,
  item, line, row_before, row_after, changed, written, dry_run, comment_posted,
  comment_url, actions? }`. It sets that row's checkbox to `[x]` and changes
  only that row line; the graph block carries no done state and is left alone.
- `tick --pr <ref>` records the delivering PR in the row's notes: a row without
  notes gets a `(PR <ref>)` group after its ref or title and before any
  `· after` clause, and a row with notes gets `, PR <ref>` appended inside the
  group. A PR the notes already name is not added again. `<ref>` must be one token without
  whitespace, parentheses, commas, or a middle dot (`tracker_pr_invalid`).
- `tick --comment-file <path>` posts that file as one issue comment after the
  body write succeeds. The comment is validated before anything is written,
  with the `issue comment` guards and error kinds and messages that name
  `--comment-file`. If
  the comment call still fails after the write, the error is
  `tracker_comment_not_posted` (`RUNTIME 1`): the row is ticked, and the caller
  posts the comment with `issue comment`, because ticking again is a no-op.
- `tick` refuses with `tracker_item_unknown`, `tracker_item_duplicated`, or
  `tracker_item_malformed` (`DATA 65`) when no row, more than one row, or a
  malformed row line carries the item id. A body without a phase table has no
  row, so every item is `tracker_item_unknown`. Ticking a row that is already ticked,
  with no new PR to record, is a no-op: `changed = false`, nothing is written,
  and no comment is posted.
- **Write rule.** Every provider write reads the issue immediately before
  writing, applies the block or row transformation to that freshly read body,
  and writes the result through the `issue edit` body path with its payload
  guards (rules 11 and 17 and the escaped-control guard). A body from an
  earlier read is never written, so a change another session made since the
  caller last looked is kept. The provider offers no compare-and-swap, so a
  write landing between this read and this write can still be lost.
- **Re-submitted text.** `graph --write` and `tick` save the whole body again,
  so text the issue's authors wrote is re-submitted under the caller's
  identity. On GitLab a saved description is interpreted for quick actions with
  the saving user's permissions. This is a known property of writing a body
  through `issue edit`; these commands do not change it.
- `--body-file <path>` on `lint` and `graph` works on a local draft: no
  provider is resolved or called, the provider findings are skipped, and
  `source = body-file` with `provider`, `number`, and `url` set to `null`.
  `--body-file -` reads stdin. `graph --write` rewrites the draft file, and
  refuses stdin with `tracker_stdin_not_writable`. `--check-state` and an issue
  id cannot be combined with `--body-file`.
- `--dry-run` on a read-only invocation (`lint`, or `graph` without `--write`)
  emits the planned `issue view` argv under `data.plan`. On `graph --write` and
  `tick` it runs the read, reports the planned change with `written = false`,
  and lists each backend call a real run would make under
  `data.actions[] = { kind, plan }`, where `kind` is `edit-body` or `comment`.

(See `forge-cli-ops-v1.yaml` for the remaining ops.)

### `comment redact`

GitHub repository write access permits editing another author's comment, as
specified in [GitHub's moderation documentation](https://docs.github.com/en/communities/moderating-comments-and-conversations/managing-disruptive-comments#editing-a-comment).
The [issue-comment REST update endpoint](https://docs.github.com/en/rest/issues/comments#update-an-issue-comment)
requires Issues or Pull requests write permission for fine-grained tokens.

`--repo <owner/name> comment redact <database-id> --inspect` reads one issue or
PR timeline comment and returns its author and lowercase SHA-256 digest of the
exact UTF-8 body, without returning the body. Inspection uses `api_read`;
redaction uses `api_write`. Both verify the selected authenticated actor,
repository identity and repository write access (`permissions.push`).
The provider remains responsible for enforcing token permissions on PATCH.

To replace that current body, use `--repo <owner/name> comment redact
<database-id> --expected-author <login> --expected-body-sha256 <digest>
--body-file <replacement.md>`. The explicit repository, positive native comment
ID, expected original author, digest, and nonempty replacement file are required.
Inline bodies and stdin are not accepted. The current author's login must match
without ASCII case sensitivity; the comment ID and issue URL must belong to the
exact repository and authority. Review diff comments are outside this command.
Normal `comment edit` retains its own-author guard. Review ledger and handoff
records carrying forge-cli review markers are protected from this operation.
Replacement review markers are refused locally before any provider read, including
in dry-run. GitHub Enterprise managed author logins containing underscores are accepted.

The command reads the body privately in an encoded provider response field so
credential scrubbing cannot alter the bytes being hashed. Provider responses,
backend diagnostics and original/replacement bodies are never printed. The
replacement passes the normal publication guards and is sent in a scoped JSON
payload file. Every backend request has a 30-second deadline.

A mismatched author, digest or target refuses before PATCH; missing repository
write access yields `comment_redaction_forbidden`. Malformed/unavailable reads
return `comment_redaction_read_failed` without provider content. Identity-preparation
policy and credential errors retain their original typed code and exit status
(for example `DATA 65` / `identity_operation_denied`). Like normal
editing, this digest check is read-then-write, **not atomic CAS**: another edit
between the GET and PATCH can be overwritten. Coordinate concurrent edits.

If the current body already equals the replacement, the command verifies the
author/target/permission and succeeds as `already_replaced` without PATCH,
regardless of the old digest. Otherwise it issues at most one PATCH, then always
performs a separate GET, including after a failed or lost PATCH response.
Only an exact replacement body with the expected author and target succeeds as
`verified`. All unverified post-write outcomes return `UNAVAILABLE 69` /
`comment_redaction_unresolved` with `retryable: false` and
`next_action: inspect_comment_and_permissions`. Inspect before deciding whether
to retry; no automatic replay, deletion, impersonation or credential fallback
occurs. An unchanged readback after a backend failure is conservatively unresolved.

Success uses `cli.forge-cli.comment.redact.v1` with `provider`, `repository`,
`id`, `author`, `body_sha256`, `status` (`inspected`, `already_replaced`, or
`verified`), and `history_redacted: false`. Text output also discloses the
history limitation. Offline `--dry-run` validates local inputs and returns
`status: planned` and the sibling-compatible `plan` array, without proving author,
permission or freshness. The plan uses a payload-file placeholder and contains no body.

**Replacing the current body does not redact edit history.** Readers can still
see earlier revisions. An authorized maintainer must separately use GitHub's
[Delete revision from history](https://docs.github.com/en/communities/moderating-comments-and-conversations/tracking-changes-in-a-comment#deleting-sensitive-information-from-a-comments-history)
flow when historical content also needs removal. This command never claims
complete privacy remediation or silently deletes a historical comment.
