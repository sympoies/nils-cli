# Session Board V1

## Status and ownership

- Status: implementation contract for the `agent-session` session board.
- Schema family: `agent-session.board*.v1`.
- Owner: `nils-agent-session` (daemon projection, closed ledger, relay route,
  and CLI). The deployment's aggregator owns cross-machine aggregation,
  retention, and the query route in [Aggregator query contract](#aggregator-query-contract).
- Program key: `agent-console-board-2026-09`, item A0. Implementation items A1
  (local projection), A2 (closed ledger), and A5 (CLI) follow this contract.
  A5 shipped in two steps: A5a is the CLI with local mode, and A5b adds the
  daemon relay route and relay mode.
- Code placement: board projection, ledger, relay, and CLI code lives in its
  own module (`crates/agent-session/src/board.rs` or a `board/` directory),
  not in `serve.rs` or `lib.rs`. Those files only register routes and the
  subcommand and call into the board module.

The board lets every session in one deployment see which other sessions
exist, where they run, and roughly what they are doing, including sessions
that closed recently. It is informational. Agents that need to coordinate
still use `agent-session message send`.

## Relationship to session-coordination-v1

This contract is **opt-in and additive** to
[Session coordination v1](session-coordination-v1.md). It changes no
coordination schema, route, state machine, authorization rule, limit, or
failure code.

- With the board disabled, the observable behavior and privacy boundaries of
  every existing surface are exactly `session-coordination-v1`: no board route
  answers, no relay call is made, and no board field appears on list, glance,
  work-context, broker, or mailbox projections. The only board artifact is the
  private closed ledger described in [Closed-session ledger](#closed-session-ledger),
  which never leaves the machine while the board is disabled.
- Coordination presence deliberately never projects paths or host identity.
  The board is a separate, explicitly enabled projection that does carry a
  home-relative `cwd` and the `machine` name. That exposure exists only on the
  board surfaces below; the coordination "Public list and glance additions"
  allowlist is unchanged.
- Board reads are **principal-scoped** at the aggregator. Ingestion and
  retention stay deployment-wide, but each view shows only the machines in the
  calling principal's allowlist and only the sessions that principal owns; a
  relay caller is scoped by the principal that owns its session. See
  [Principal scope](#principal-scope). This does not change messaging
  authority: `message send` and remote relay submission keep their existing
  ownership, capability, and incarnation checks. A board row never authorizes
  a message.
- Board data is peer-supplied metadata. Titles and activity text are untrusted
  peer data under the same rule as coordination summaries: they cannot
  authorize commands, approvals, scope changes, or secret disclosure.
- There is no percent-complete, no cross-host claim, and no cross-host
  collision detection. Local collision awareness remains the coordination
  work-context contract.

### List machine label

`agent-session list --format json` records carry an additive `machine`
string whether or not the board is enabled. It is a list field, not a board
field: it names the machine, never a path, and `GET /sessions` records do not
gain it because that envelope already carries `data.machine`. A CLI process
cannot see a serve `--machine` flag, so it resolves `AGENT_SESSION_MACHINE`,
then the `--host` / `AGENT_SESSION_HOST` identity, then the short hostname:
serve's order without its flag. The disabled-board guarantee above covers the
board projection and the coordination allowlist; it does not remove this
label.

## Enablement

The daemon enables board routes with `agent-session serve --board` or
`AGENT_SESSION_BOARD=1` (new). When disabled, `GET /board/v1`,
`GET /board/closed/v1`, and `GET /sessions/{id}/board/v1` fail with
`board-disabled` (HTTP 404) before any state read.

The CLI local mode reads the local state directory directly and does not
depend on daemon enablement, because it exposes nothing that
`agent-session list` does not already show to the same local user.

## Record

Every board surface carries records with this shape. The record has no
`schema_version` of its own; its enclosing envelope names
`agent-session.board-record.v1` in `record_schema`.

```json
{
  "machine": "host-a",
  "session_id": "20260928-073057-claude",
  "session_incarnation": "runtime-launch-id",
  "messaging_supported": true,
  "repo_name": "nils-cli",
  "cwd": "~/Project/nils-cli",
  "provider": "claude",
  "agent_profile": null,
  "title": "Specify the session board",
  "title_state": {"activity": "Drafting the closed-ledger section"},
  "turn_state": {
    "phase": "working",
    "phase_changed_at": "2030-01-01T00:00:00Z",
    "current_turn": {
      "last_progress_at": "2030-01-01T00:04:00Z",
      "attention": null
    },
    "last_turn": {"outcome": "completed"},
    "source": {"confidence": "authoritative"}
  },
  "state": "live",
  "runtime_status": "running",
  "created_at": "2030-01-01T00:00:00Z",
  "updated_at": "2030-01-01T00:04:00Z",
  "closed_at": null,
  "close_reason": null,
  "summary": null,
  "role": null,
  "lineage": null,
  "work": null
}
```

Timestamps are RFC 3339 UTC strings, as in `SessionView`. Every field is
always present; an unavailable value is `null`, never omitted. The optional
members are the aggregator annotations `console_owner`, `orphaned`, and
`subtree` described in [Aggregator annotations](#aggregator-annotations),
which are not board-record fields.

`role`, `lineage`, and `work` are additive `lineage.v1` and `work.v1`
extensions (see [Extensions](#extensions)). Each is `null` when the session
has none, or when the record comes from a daemon or ledger entry that predates
the extension. A session with lineage and work looks like this:

```json
{
  "role": "coordinator",
  "lineage": {
    "parent": {"machine": "host-a", "session_id": "parent", "session_created_at": "2030-01-01T00:00:00Z"},
    "effective_parent": {"machine": "host-a", "session_id": "parent", "session_created_at": "2030-01-01T00:00:00Z"},
    "root": {"machine": "host-a", "session_id": "root", "session_created_at": "2030-01-01T00:00:00Z"},
    "depth": 2,
    "starter": {"kind": "session", "via": "console"},
    "budget": null
  },
  "work": {
    "program": {"provider": "github", "repository": "owner/program", "number": 44},
    "issues": [{"provider": "github", "repository": "owner/repo", "number": 2032}],
    "inherited": true
  }
}
```

### Field sources

"`SessionView`" means the per-session projection that `agent-session list` and
`GET /sessions` already build. "New" marks a value this contract introduces.

| Group | Field | Type | Source |
| --- | --- | --- | --- |
| Identity | `machine` | string | New per-record field, stamped by the process that serves the record, never stored: on daemon routes it is the existing serve machine identity (`--machine`, `AGENT_SESSION_MACHINE`, `--host`, short hostname), also reported in the serve envelope `data.machine`; in CLI local mode see [Mode selection](#mode-selection). It always equals the enclosing envelope `machine`. |
| Identity | `session_id` | string | `SessionView.id`. |
| Identity | `session_incarnation` | string or null | `SessionView.session_incarnation`; null when the record has no current runtime launch. |
| Identity | `messaging_supported` | bool | New derivation, same meaning as `messaging_supported` in `agent-session.remote-peers.v1`: this session can currently receive a remote message. See [Messaging support](#messaging-support). |
| Place | `repo_name` | string or null | `SessionView.repo_name`, which is the final path component of the session `cwd`. It is not an `owner/name` origin. |
| Place | `cwd` | string or null | New derivation from `SessionView.cwd`: the daemon user's home prefix is replaced by `~` (for example `~/Project/x`). A `cwd` outside that home is `null`, never an absolute path. |
| Place | `provider` | string | `SessionView.agent` (for example `codex`, `claude`, `dsh`). |
| Place | `agent_profile` | string or null | `SessionView.agent_profile`. |
| Doing | `title` | string or null | `SessionView.title`. |
| Doing | `title_state.activity` | string or null | `SessionView.title_state.activity`. `title_state` is `null` when retitle is disabled or unavailable (`SessionView.title_state` absent); otherwise it contains only `activity`. |
| Progress | `turn_state` | object or null | `SessionView.turn_state` (the [turn-state contract](../turn-state-contract.md)), projected to the allowlisted fields below. `null` when the session has no turn state. |
| Progress | `turn_state.phase` | string | `TurnState.phase`: `starting`, `working`, `waiting`, `needs_input`, or `unknown`. |
| Progress | `turn_state.phase_changed_at` | string | `TurnState.phase_changed_at`. |
| Progress | `turn_state.current_turn` | object or null | `TurnState.current_turn`, projected to `last_progress_at` and `attention`. |
| Progress | `turn_state.current_turn.last_progress_at` | string or null | `CurrentTurn.last_progress_at`. |
| Progress | `turn_state.current_turn.attention` | object or null | `CurrentTurn.attention`, projected to `kind` and `requested_at`. |
| Progress | `turn_state.last_turn` | object or null | `TurnState.last_turn`, projected to `outcome` only. |
| Progress | `turn_state.source.confidence` | string | `TurnState.source.confidence`: `authoritative`, `observed`, or `inferred`. |
| Lifecycle | `state` | string | New derivation: `live`, `stopped`, or `closed`. See [Lifecycle states](#lifecycle-states). |
| Lifecycle | `runtime_status` | string or null | `SessionView.status` (`running`, `stopped`, `missing`, or `unknown`) at projection time; `null` on a closed record. New on the board so a consumer can tell a proven stop from an unverified probe. |
| Lifecycle | `created_at` | string | `SessionView.created_at`. |
| Lifecycle | `updated_at` | string | `SessionView.updated_at`; on a closed record, the last value before close. |
| Lifecycle | `closed_at` | string or null | New. The ledger append time for `deleted` or `archived`; the aggregator's first confirmed absence for `vanished`. `null` unless `state` is `closed`. |
| Lifecycle | `close_reason` | string or null | New. `deleted`, `archived`, `exited`, or `vanished`. See [Close reasons](#close-reasons). `null` unless `state` is `closed`. |
| Role | `role` | string or null | `SessionView.role`: `coordinator`, or `null`. See [session-lineage-work-v1](session-lineage-work-v1.md#role). |
| Lineage | `lineage` | object or null | `SessionView.lineage` and `SessionView.lineage_adoption`, projected to `{parent, effective_parent, root, depth, starter, budget}`; `null` for a session started before lineage existed. References are `{machine, session_id, session_created_at}` and never carry an incarnation. `parent` is `null` for a root; `effective_parent` is the adopting steward when `lineage adopt` named one, otherwise `parent`; `root` is the session itself for a root; `depth` is 0 for a root; `starter` is `{kind, via}`; `budget` is the stored subtree budget, or `null`. See [session-lineage-work-v1](session-lineage-work-v1.md#lineage). |
| Work | `work` | object or null | `SessionView.work`, projected to `{program, issues, inherited}` (provider references `{provider, repository, number}`); `null` for a session with no program or issue. The revision is not exposed. See [session-lineage-work-v1](session-lineage-work-v1.md#work). |
| Reserved | `summary` | string or null | Reserved for the `agent-session.work-context.v1` `summary` (at most 240 UTF-8 bytes). v1 producers always emit `null`; v1 consumers accept a string or `null` and must not depend on it. |

Any `SessionView` field not in this table is excluded, in particular
`last_prompt`, transcripts, pane content, `attach_command`,
`ssh_attach_command`, `tmux_session`, `prompt_file`, `log_file`, provider
resume identity, Codex account data, work-context scopes and claims, mailbox
counts, capabilities, and orchestration projections. The bounded `role`,
`lineage`, and `work` objects above replace none of these: they name
sessions and public provider references only. Unknown upstream
`turn_state` fields are dropped, not passed through.

### Lifecycle states

- `live`: the record exists and its runtime is running (`SessionView.status`
  is `running`).
- `stopped`: the record still exists but no runtime is proven running
  (`SessionView.status` is `stopped`, `missing`, or `unknown`). The session may
  be resumable. `runtime_status` distinguishes a proven stop from a failed or
  timed-out probe.
- `closed`: the record was removed and now exists only as a closed-ledger
  entry or an aggregator-retained row.

A resume keeps `session_id` and `created_at` and rotates
`session_incarnation`, so the same board row moves from `stopped` back to
`live`. The row identity is `(machine, session_id, created_at)`; a later
record that reuses a `session_id` has a different `created_at` and is a
different row.

### Close reasons

| Reason | Writer | Meaning |
| --- | --- | --- |
| `deleted` | daemon ledger | The record was removed by delete (CLI, `DELETE /sessions/{id}`, group cleanup, or the maintenance `remove_console_record` action). |
| `archived` | daemon ledger | The record was removed by archive (`POST /sessions/{id}/archive` or group archive), which also wrote history-archive metadata. |
| `exited` | none in v1 | Reserved. v1 producers never write it; consumers accept it. See [Decision: runtime exit is not a close](#decision-runtime-exit-is-not-a-close). |
| `vanished` | aggregator only | The record disappeared from a reachable machine's snapshot with no ledger entry. See [Vanished records](#vanished-records). A daemon never writes it. |

### Decision: runtime exit is not a close

Settled here for item A2: a runtime that exits while its record remains does
**not** write a ledger entry. The session stays in `GET /board/v1` as
`stopped`. Only removal of the record writes a ledger entry.

Rationale:

1. One surface per record. `GET /board/v1` describes every record that exists,
   and the ledger describes only records that no longer exist. An `exited`
   entry would put the same session in both, with contradictory states.
2. Resume would falsify it. A stopped session can be resumed under the same
   `session_id` and `created_at`. A closed entry would then need retraction
   semantics that the append-only ledger does not have.
3. Nothing is lost. The ledger exists so the aggregator can catch up after
   downtime. An exited session's record is still present at catch-up, with
   `state: stopped`, `runtime_status`, and `updated_at`, so the aggregator sees
   the exit without a ledger entry.
4. No write point exists. Stopped status is derived at read time from the
   runtime probe. Writing `exited` would need a new exit watcher across every
   runtime kind, which is outside this program.

`exited` stays in the closed enum so that a later revision can emit it for a
case that meets these rules without a consumer-visible enum change.

### Messaging support

The daemon sets `messaging_supported` to `true` only when all hold:

- federation is configured on this daemon (`AGENT_SESSION_RELAY_URL`,
  `AGENT_SESSION_RELAY_TOKEN`, and `AGENT_SESSION_RELAY_INGRESS_TOKEN`, the
  same condition as `data.coordination.remote_messaging_supported`);
- `state` is `live` and `session_incarnation` is non-null;
- `coordination_mode` is not `off` and `coordination_available` is `true`.

The aggregator may additionally require its own registration check, so that
the aggregated value has the exact `agent-session.remote-peers.v1` meaning. The
value describes whether the target can receive, never whether the caller may
send; a cross-principal send is still rejected by the relay ownership checks.

## Extensions

The v1 envelopes (`agent-session.board.v1`, `agent-session.board-closed.v1`,
and `agent-session.board-view.v1`) carry `extensions`, an array of strings
naming the additive capabilities the producer supports:

| Extension | Adds |
| --- | --- |
| `lineage.v1` | Record `role` and `lineage`; aggregator annotations `orphaned` and `subtree`; the `root` query filter. |
| `work.v1` | Record `work`. |
| `programs.v1` | The daemon route `GET /board/programs/v1` and the aggregator view `programs` member. Named by the daemon snapshot only. |

A reader feature-detects from this list, never from the presence of a record
field, and ignores strings it does not know. A producer that predates an
extension omits the list, and its records carry no such field; a reader then
treats the field as `null`. Adding an extension never changes the schema
strings, so a deployed reader that requires the exact v1 strings keeps working.

## Daemon local snapshot

`GET /board/v1` returns the daemon's local `live` and `stopped` records.

- Authority: the server operator bearer (`AGENT_SESSION_TOKEN` or
  `--token-stdin`), unlike the open `GET /sessions`. Session capabilities are
  not accepted as a substitute.
- Envelope: the ordinary `cli.agent-session.serve.v1` success envelope with
  `data.machine` and `data.board`.

```json
{
  "schema_version": "agent-session.board.v1",
  "record_schema": "agent-session.board-record.v1",
  "machine": "host-a",
  "extensions": ["lineage.v1", "work.v1", "programs.v1"],
  "generated_at": "2030-01-01T00:05:00Z",
  "ledger_cursor": "opaque",
  "records": [],
  "skipped_count": 0
}
```

`ledger_cursor` is the closed-ledger head read **before** records are
enumerated. Any record removed after that point therefore appears in
`GET /board/closed/v1?since=<ledger_cursor>`. A consumer uses it only as the
starting closed cursor when it holds none for that machine (first contact or
[Full resync](#full-resync)); otherwise it keeps advancing its stored cursor
from `next_cursor` and ignores `ledger_cursor`, because replacing a stored
cursor that is behind would skip entries. When the ledger head cannot be read
(lock timeout or untrusted store), the snapshot fails with
`board-ledger-unavailable` rather than omitting the cursor. Records are sorted by
`session_id` ascending. The route takes no filters: the aggregator filters.
A record whose projection fails (corrupt or unreadable) is omitted and counted
in `skipped_count`; it is never guessed. The aggregator treats a snapshot with
`skipped_count > 0` as incomplete for [Vanished records](#vanished-records).

## Programs

A session's `work.program` names a work-mode tracker issue. The daemon serves
the lanes of every program that a session on this machine names, so a board can
group sessions by lane without reading the forge itself.

`GET /board/programs/v1` has the authority and envelope of `GET /board/v1`,
with the result in `data.board_programs`:

```json
{
  "schema_version": "agent-session.board-programs.v1",
  "machine": "host-a",
  "generated_at": "2030-01-01T00:05:00Z",
  "programs": [
    {
      "ref": "owner/program#44",
      "url": "https://github.com/owner/program/issues/44",
      "title": "Program tracker",
      "state": "open",
      "fetched_at": "2030-01-01T00:04:00Z",
      "stale": false,
      "rows": [
        {"id": "A1", "title": "First lane", "reference": "owner/repo#2032", "done": false, "phase": "Phase 1", "after": [], "notes": null},
        {"id": "G1", "title": "Release gate", "reference": null, "done": false, "phase": null, "after": ["A1"], "notes": null}
      ]
    }
  ]
}
```

- `ref` is the program in the `owner/repo#N` grammar, with a `gitlab:` prefix
  for GitLab. `programs` lists the distinct programs named by this machine's
  sessions, sorted by `ref`, at most 16.
- A row is a lane when `reference` is set: its sessions are those whose
  `work.issues` contain that reference. A row without a reference is a gate.
  `after` lists the ids of the rows it waits for, and `done` is the tracker's
  tick. Rows come from `forge-cli issue tracker show`, which owns the tracker
  grammar; row findings are not served.
- The daemon reads each program through `forge-cli issue tracker show` (the
  binary named by `AGENT_SESSION_FORGE_CLI_BIN`, otherwise `forge-cli` on
  `PATH`) lazily when the route is read. A copy younger than five minutes is
  served without a read, concurrent reads share one refresh, and the last good
  copy is kept with `stale: true` when a refresh fails. A failed read counts
  as an attempt: it is not tried again before the five minutes pass. One
  refresh pass reads for at most 20 seconds in total, each read for at most 10;
  programs it does not reach keep their cached copy, marked `stale: true`. A program that was
  never read successfully is omitted, never guessed. Program refs and the
  tracker's public title, state, and rows are the only data served.
- The route answers `board-disabled` (HTTP 404) while the board is disabled.

An aggregator merges the programs of every machine by `ref`, keeping the newest
`fetched_at`, and adds them to its view as `programs`, restricted to the
programs that records visible to the principal name in `work.program`.

## Closed-session ledger

### Storage

The ledger is a private file `<state-dir>/board/closed-ledger.json`, schema
`agent-session.board-closed-ledger.v1` (new). `<state-dir>/board` is mode
`0700` and the file is `0600`, owned by the current user, not a symlink, and
canonically below the state directory, using the same trust checks as the
coordination root. Writes use atomic replace. One bounded lock (2-second
timeout, as for the coordination registry) serializes appends, pruning, and
reads.

```json
{
  "schema_version": "agent-session.board-closed-ledger.v1",
  "ledger_id": "uuid",
  "last_seq": 42,
  "entries": [{"seq": 42, "record": {}}]
}
```

`ledger_id` is a random UUID minted when the ledger file is created. `seq`
starts at 1 and increases by 1 per append under that `ledger_id`. Each
`record` is a [closed record](#closed-record-values).

### Closed record values

A closed record, whether written to the ledger or produced by the aggregator's
[Vanished records](#vanished-records) conversion, has:

- `state: closed`, and `closed_at` and `close_reason` as defined in
  [Field sources](#field-sources);
- `runtime_status: null` and `messaging_supported: false`, whatever the
  removed record last showed;
- every other field set to the last value the removed record held (for a
  vanished row, the last value the aggregator ingested).

Ledger entries do not store `machine`, because they are also written by CLI
processes that cannot see the serve identity. The daemon stamps each served
closed record with its own serve identity, as for snapshot records.

### Append

Every path that removes a session record through the shared logical deletion
appends exactly one entry after the removal commits: `deleted` for delete,
group cleanup, and `remove_console_record`; `archived` for archive and group
archive. The ledger is written whether or not the board is enabled, because
deletions also happen in CLI processes that cannot see daemon configuration;
gating the write would leave silent gaps. It is a private file and is exposed
only through the enabled routes.

A ledger failure never fails, delays, or rolls back the deletion. The failure
is recorded content-free, and the aggregator later classifies the record as
`vanished`. An unreadable, corrupt, or unsupported-version ledger file is
moved aside and replaced by an empty ledger with a new `ledger_id` at the
next append **or read**, under the ledger lock, so earlier cursors become
`board-cursor-expired` and reads recover without waiting for a deletion.
`board-ledger-unavailable` is reserved for a lock timeout or an untrusted
store (wrong owner, mode, symlink, or location), which is never repaired
automatically.

### Bound

An entry is retained while both hold: it is at most 7 days old by `closed_at`,
and it is among the newest 256 entries. Whichever bound is smaller wins.
Pruning runs on every append and read. Eviction is safe here, unlike
coordination receipts, because a consumer that missed evicted entries is told
so by `board-cursor-expired`.

Closed records keep `role`, `lineage`, and `work`, so a tree view can show
recently closed children. A ledger entry written before those members existed
is served with them as `null`.

### Read

`GET /board/closed/v1?since=<cursor>` returns retained entries with `seq`
greater than the cursor's, in ascending `seq` order. Omitting `since` returns
every retained entry. Authority and envelope match `GET /board/v1`, with the
result in `data.board_closed`:

```json
{
  "schema_version": "agent-session.board-closed.v1",
  "record_schema": "agent-session.board-record.v1",
  "machine": "host-a",
  "extensions": ["lineage.v1", "work.v1"],
  "generated_at": "2030-01-01T00:05:00Z",
  "entries": [{"cursor": "opaque", "record": {}}],
  "next_cursor": "opaque"
}
```

- A cursor is opaque. Consumers store and return it and never parse it. It
  binds `ledger_id` and `seq`.
- `next_cursor` is the cursor of the last returned entry, or the ledger head
  when none is returned. It is always valid for the next request, including
  on an empty ledger.
- Order is by `seq` only. `closed_at` is wall-clock time and is not a
  guaranteed ordering key.
- The retained ledger holds at most 256 entries, so one response always
  returns the whole tail. There is no pagination in v1.
- A malformed cursor (not one this daemon could have issued) fails with
  `board-cursor-invalid` (HTTP 400). This is the only invalid case.
- A well-formed cursor fails with `board-cursor-expired` (HTTP 410) when its
  `ledger_id` differs from the current one, when its `seq` is ahead of the
  ledger head (for example after the file was restored from an older copy),
  or when entries after it were pruned. The `ledger_id` comparison runs
  first. Every expired case sends the consumer to [Full resync](#full-resync).

### Full resync

On `board-cursor-expired` the consumer discards its cursor and resynchronizes:

1. `GET /board/closed/v1` without `since`, applying every retained entry and
   storing its `next_cursor`;
2. `GET /board/v1`, replacing its whole view of that machine's live and
   stopped records;
3. applying [Vanished records](#vanished-records) to any row it held for that
   machine that is in neither response.

## Relay route

`GET /sessions/{id}/board/v1?state=&since=&repo=&machine=&root=` forwards a board
query from a managed session to the aggregator.

- Authority: the current local session capability in
  `Authorization: Bearer`, exactly as `GET /sessions/{id}/messages/peers/v1`.
  The daemon authenticates the exact current incarnation. Operator authority is
  not a substitute.
- Configuration: the existing federation values `AGENT_SESSION_RELAY_URL` and
  `AGENT_SESSION_RELAY_TOKEN`. The daemon calls
  `GET {AGENT_SESSION_RELAY_URL}/api/coordination/board/v1` with the relay
  token as bearer, forwarding the five filters unchanged and adding
  `source_session_id` and `source_incarnation`, as peer discovery does. The
  board adds **no new secret**: the ingress token keeps its existing meaning
  and is not used by board routes, and the existing rule that the operator
  token differs from both relay tokens still applies.
- Result: the raw `agent-session.board-view.v1` JSON from the aggregator,
  after the daemon checks its `schema_version`. Failures use the serve error
  envelope.
- With federation unconfigured, the route fails with `board-relay-disabled`
  (HTTP 409) and makes no network call. An aggregator
  [scope refusal](#principal-scope) keeps its own code with a fixed message
  and the workspace data exit class: a 403 `ownership-unknown` or
  `machine-forbidden` fails with that code (HTTP 422), and a 409
  `session-incarnation-conflict` fails with that code (HTTP 409). Any other
  aggregator 401 or 403 fails with `board-relay-unauthorized` (HTTP 502). An
  aggregator 400 whose
  [failure body](#aggregator-failures) carries `error.code`
  `board-query-invalid` is passed through as `board-query-invalid` (HTTP 400),
  forwarding `error.message` when it is a bounded single-line string and a
  fixed message otherwise. Network failure, any other non-success status or
  code, an unreadable failure body, or an unexpected success schema fails with
  `board-relay-unavailable` (HTTP 502).
- No lock is held across the network call. The CLI never reads relay
  secrets; it reaches this route only through the private
  `coordination/daemon-endpoint.json`, as federated messaging does.

## Aggregator query contract

The deployment's aggregator implements
`GET /api/coordination/board/v1`. This section is the contract the daemon
relay route and the CLI rely on. Aggregator storage and ingestion scheduling
are outside this contract.

### Ingestion obligations

- Ingest from every configured machine, including machines with no relay
  configuration, using `GET /board/v1` and `GET /board/closed/v1` with that
  machine's operator bearer.
- Keep one opaque closed cursor per machine, advanced only from
  `next_cursor`, and follow [Full resync](#full-resync) on
  `board-cursor-expired`. A `ledger_id` change always surfaces as
  `board-cursor-expired`; the aggregator never parses a cursor.
- Bind each response to the configured machine it was fetched from. A
  snapshot or closed read whose envelope `machine`, or any record `machine`,
  differs from that configured name is a failed ingestion attempt and is not
  applied.
- Treat a machine as unavailable when its last ingestion attempt failed or its
  last success is older than the aggregator's freshness window. Retain its
  `last_seen_at` (time of the last successful snapshot).

### Vanished records

A row becomes a [closed record](#closed-record-values) with
`close_reason: vanished` only when all hold:

- its machine is available;
- the row is absent from two consecutive successful snapshots taken at least
  60 seconds apart;
- no ledger entry for the same `(machine, session_id, created_at)` arrived
  through a closed-ledger read that started after the first of those
  snapshots.

The two-snapshot rule covers a deletion whose ledger append has not landed
yet. `closed_at` is the time of the first snapshot that lacked the row. A row
never vanishes while its machine is unavailable. If a vanished row reappears
with the same identity, the aggregator restores it. A later ledger entry for
the same identity replaces `vanished` with the ledger reason.

### Query

`GET /api/coordination/board/v1` requires the relay bearer of a configured
source machine. `source_session_id` and `source_incarnation` identify the
caller, and the aggregator scopes the result to the principal that owns that
session, as described in [Principal scope](#principal-scope).

| Parameter | Values | Default | Rule |
| --- | --- | --- | --- |
| `state` | `live`, `stopped`, `closed`, `all` | `all` | Exact match on record `state`. |
| `since` | duration `<n><unit>`, unit `m`, `h`, `d`, `w`, or `mo` (31 days) | the configured retention | Keeps `stopped` rows by `updated_at` and `closed` rows by `closed_at` within the window. `live` rows are always kept. |
| `repo` | string | none | Exact, case-sensitive match on `repo_name`. |
| `machine` | string | none | Exact match on `machine`. |
| `root` | session id | none | Only the session tree of that root: the record whose `session_id` is the id, and every record whose `lineage.root.session_id` is the id. Needs `lineage.v1`. |

`since` is capped by the aggregator's configured retention. The default
retention is `3d`, and a deployment may configure `7d`, `2w`, or `1mo`
(31 days, the maximum). A larger `since` is clamped, not rejected. An unknown
parameter, repeated parameter, or invalid value fails with
`board-query-invalid` (HTTP 400).

### Aggregator failures

Aggregator failures use the same JSON error body as the existing federation
edge routes (for example `/api/coordination/peers/v1`): the stable code at
`error.code` and an optional bounded, single-line `error.message` that never
echoes a token, capability, or absolute path. A missing or invalid relay
bearer is HTTP 401 or 403.

### View

```json
{
  "schema_version": "agent-session.board-view.v1",
  "record_schema": "agent-session.board-record.v1",
  "extensions": ["lineage.v1", "work.v1"],
  "generated_at": "2030-01-01T00:05:00Z",
  "retention": "3d",
  "effective_since": "2029-12-29T00:05:00Z",
  "since_capped": false,
  "machines": [
    {"machine": "host-a", "available": true, "last_seen_at": "2030-01-01T00:04:55Z"},
    {"machine": "host-b", "available": false, "last_seen_at": "2029-12-31T22:10:00Z"}
  ],
  "records": [],
  "truncated": false
}
```

- `machines` lists every configured machine, sorted by `machine`, whether or
  not the `machine` filter matches it. The unavailable-machine projection is
  exactly `machine`, `available: false`, and `last_seen_at` (null if never
  seen).
- An unavailable machine contributes no `live` or `stopped` rows, because they
  cannot be verified. Its already ingested `closed` rows remain.
- `records` are sorted by state (`live`, then `stopped`, then `closed`), then
  by `updated_at` (for `live` and `stopped`) or `closed_at` (for `closed`),
  newest first, then by `machine` and `session_id` ascending.
- At most 1024 records are returned; `truncated: true` reports that more
  matched.
- A view whose `extensions` name `programs.v1` also carries `programs`, the
  merged [Programs](#programs) list; a view without the extension has no
  `programs` member.
- `retention` is the aggregator's configured retention (`3d`, `7d`, `2w`, or
  `1mo`). `effective_since` is the start of the window actually applied.
  `since_capped` is `true` exactly when the requested `since` was longer than
  `retention` and was clamped to it.

### Since beyond retention

A `since` longer than the configured retention is clamped and reported with
`since_capped: true`; it is not an error. v1 defines no `retention-exceeded`
failure code, and an aggregator must not return one. A consumer that needs to
know the real window reads `retention` and `effective_since`.

### Aggregator annotations

An aggregator view may add one optional member to each record:
`console_owner` (string or null), the aggregator's display label for the
deployment principal that owns the session. It is an annotation, not a
board-record field:

- daemons and CLI local mode never emit it; the relay route passes it through
  unchanged when the aggregator sends it;
- absence and `null` both mean unknown;
- it is display-only. It never authorizes a message, and a consumer must not
  infer from it, or from `messaging_supported`, that the caller may message
  the session. `message send` ownership checks decide that.

Whether an aggregator populates it is the aggregator's choice.

With `lineage.v1` an aggregator view, and CLI local mode for its one machine,
may add two more annotations:

- `orphaned` (bool): `true` on a non-closed record whose `lineage.effective_parent`
  is a closed record, or is absent from every live machine, so the tree still
  groups it under its `root` after an intermediate session disappears. A
  record with no effective parent is not orphaned. Children keep their `root`.
  CLI local mode sees one machine, so it judges only a parent that is closed
  or named with that machine's label; a parent on another machine is never
  reported as missing from here.
- `subtree` (`{live, stopped}`): on a record that is its own root, or a
  session without lineage that other records name as their root, the number
  of other non-closed records whose `lineage.root` is it.

Both are display-only and never authorize anything. Like `console_owner`, a
daemon never emits them; the relay route passes them through unchanged.

### Principal scope

The aggregator stores every record from every configured machine, whoever
owns it, and applies retention to all of them. Scoping happens when a view is
built, for one principal:

- The view names only machines in that principal's machine allowlist, in
  `machines` and in `records`. Any other configured machine is not mentioned,
  not even as unavailable, and a `machine` filter naming it returns no
  records, exactly like a machine that does not exist.
- The view returns only records whose session the aggregator's ownership
  store attributes to that principal. A session it cannot attribute is hidden
  on every machine.
- The filter applies before the record limit, so other principals' rows never
  set `truncated`. `since_capped`, `retention`, and the record schema are
  unchanged.
- On the relay route the principal is the owner of the exact calling session
  (`source_session_id` and `source_incarnation` on the relay's machine). A
  caller the aggregator cannot scope is refused and receives no view:
  `ownership-unknown` (403) when no owner is known, `session-incarnation-conflict`
  (409) when the owner is known for another incarnation, and
  `machine-forbidden` (403) when the aggregator does not permit a board view
  from the relay's machine, for example because the owner may not use it.
- A deployment with a single operator and no principals may serve the
  unscoped view.

### Console UI surface

An aggregator may also serve the same `agent-session.board-view.v1` object to
its own user interface behind its normal user authentication, with the same
filters, clamping, failure codes, and principal scope, using the
authenticated user's principal. The route
path, its response envelope, and how the aggregator advertises that the route
exists are the aggregator's own API and are outside this contract. The daemon
and CLI never call that route.

## CLI

```text
agent-session board [--state live|stopped|closed|all] [--since <dur>] [--repo <name>] [--machine <name>] [--root <session-id>] [--format text|json]
```

Filters have the meanings in [Query](#query). `--state` defaults to `all`;
an omitted `--since` means the full retained window of the source.

### Mode selection

- **Relay mode** when the CLI runs in a managed session (trusted
  `AGENT_SESSION_ID` and `AGENT_SESSION_CAPABILITY_FILE`), the daemon endpoint
  file exists, and `GET /sessions/{id}/board/v1` succeeds.
- **Local mode** otherwise: no managed identity, no daemon endpoint, or the
  daemon answers `board-disabled` or `board-relay-disabled`. That is the case
  "no relay is configured".
- Any other relay failure is returned as that error. The CLI never falls back
  to local mode silently, because a local view would present one machine as
  the whole deployment. A daemon that does not answer behind an existing
  endpoint file is such a failure (`board-relay-unavailable`), and so is a
  claimed managed identity (`AGENT_SESSION_ID` with a capability file) that
  fails authentication while the endpoint exists, which keeps its own code
  (for example `coordination-unauthorized`). The CLI
  forwards the daemon's `error.code` and `error.message` when each is a
  bounded, single-line string, and `board-relay-unavailable` with a fixed
  message otherwise.

Local mode builds the same records as `GET /board/v1` from the local state
directory, adds closed rows from the local ledger, and applies the filters
itself. Because a CLI process cannot see daemon configuration, local mode
takes `machine` as the [list label](#list-machine-label) does and
always emits `messaging_supported: false`. `machines` has one available entry for the local machine, `retention`
is `7d` (the ledger bound), and `since` is clamped to it.

### Output

JSON uses the `cli.agent-session.board.v1` envelope:

```json
{
  "schema_version": "cli.agent-session.board.v1",
  "ok": true,
  "data": {"mode": "relay", "board": {"schema_version": "agent-session.board-view.v1"}}
}
```

`data.mode` is `local` or `relay`. `data.board` is always an
`agent-session.board-view.v1` object. Text output prints the mode, then one
line per unavailable machine, then one line per record with state, machine,
session ID, repo name, turn phase, age of `last_progress_at` (or of
`phase_changed_at` when absent), and title. The caller's own session (same
`session_id` and `session_incarnation`) ends with `(this session)`; any other
record with `messaging_supported: true` ends with its `message send` target,
`send: --to-machine <machine> --to <session_id> (incarnation <session_incarnation>)`.
A record on the viewer's own machine omits `--to-machine` because that send uses
the local mailbox: `send: --to <session_id> (incarnation <session_incarnation>)`.
The target names the exact identifiers and is omitted when one cannot be
printed exactly (whitespace, control characters, or over 256 bytes). Other
text truncates for width and never prints `summary` or `console_owner`.

Usage errors exit 64, data and contract errors use the workspace data exit
code, and runtime, storage, and relay failures use the runtime exit code, as
in coordination v1.

## Schemas

| Schema | Carried by |
| --- | --- |
| `agent-session.board-record.v1` | Named in `record_schema` of every envelope below |
| `agent-session.board.v1` | `data.board` of `GET /board/v1` |
| `agent-session.board-closed.v1` | `data.board_closed` of `GET /board/closed/v1` |
| `agent-session.board-programs.v1` | `data.board_programs` of `GET /board/programs/v1` |
| `agent-session.board-closed-ledger.v1` | Private ledger file only; never served |
| `agent-session.board-view.v1` | Aggregator query, relay route (raw), and CLI `data.board` |
| `cli.agent-session.board.v1` | CLI JSON envelope |

Unsupported schema versions fail closed. Consumers ignore unknown additive
fields on envelopes and records, but a record field listed above never changes
type within v1.

## Stable failure codes

| Code | HTTP | Where |
| --- | --- | --- |
| `board-disabled` | 404 | Daemon board and relay routes when the board is disabled |
| `board-query-invalid` | 400 | Aggregator query, relay route, CLI filter validation |
| `board-cursor-invalid` | 400 | Closed-ledger read with a malformed cursor |
| `board-cursor-expired` | 410 | Closed-ledger read; triggers [Full resync](#full-resync) |
| `board-ledger-unavailable` | 503 | Snapshot or closed-ledger read on lock timeout or untrusted store |
| `board-relay-disabled` | 409 | Relay route with federation unconfigured |
| `board-relay-unavailable` | 502 | Relay network failure, non-success, or unexpected schema |
| `board-relay-unauthorized` | 502 | Aggregator rejected the relay credential |
| `ownership-unknown` | 422 | Relay route; the aggregator cannot attribute the calling session |
| `machine-forbidden` | 422 | Relay route; the aggregator does not permit a board view from this machine |

Relay capability failures reuse `coordination-unauthorized` and
`session-incarnation-conflict`; an aggregator `session-incarnation-conflict`
refusal is passed through as the same code (HTTP 409). The CLI exits with the
workspace data exit code for all four. Missing operator bearer on the daemon
routes reuses the existing serve authentication failure. Errors never echo a
token, capability, cursor internals, or absolute path.

## Validation matrix

Implementation items must cover:

- every field source in [Field sources](#field-sources), including `null`
  `title_state`, absent `turn_state`, and unknown upstream `turn_state` fields
  being dropped;
- the exact record key set and allowlisted `turn_state` subkeys, on a fixture
  where every excluded `SessionView` field is populated, with unavailable
  values `null` rather than omitted;
- `cwd` home-relative projection, including a `cwd` outside home projecting
  `null`;
- the `live` and `stopped` mapping for every `SessionView.status` value;
- a corrupt record omitted from the snapshot and counted in `skipped_count`,
  with the other records still returned in order;
- `GET /board/v1` and `GET /board/closed/v1` rejecting a missing operator
  bearer and a session capability, and the relay route rejecting operator
  authority;
- a disabled board: all three routes return `board-disabled`, and existing
  list, glance, work-context, broker, and mailbox outputs are byte-identical;
- ledger append for delete, archive, group cleanup, group archive, and
  `remove_console_record`, including a CLI-process delete while the board is
  disabled, and no entry on runtime exit;
- a CLI-written ledger entry served by a daemon whose `--machine` differs from
  the short hostname carrying the daemon's `machine`;
- a ledger entry for a session deleted while running carrying
  `runtime_status: null` and `messaging_supported: false`;
- ledger failure not failing deletion, and a corrupt or unsupported-version
  ledger replaced with a new `ledger_id` on read as well as on append;
- the snapshot `ledger_cursor` read before enumeration: a record removed
  between the two steps (through a test hook, not timing) appears in
  `GET /board/closed/v1?since=<ledger_cursor>`;
- the 7-day and 256-entry bounds, cursor order, an empty-ledger `next_cursor`
  round trip, `board-cursor-invalid` only for a malformed cursor, and
  `board-cursor-expired` after pruning, after a `ledger_id` change (checked
  before the `seq` bound), and for a same-ledger cursor ahead of the head;
- relay route capability authentication, `board-relay-disabled` without a
  network call, and error mapping, including an aggregator
  `error.code` `board-query-invalid` passed through and any other failure
  mapped to `board-relay-unavailable`;
- CLI mode selection, including no silent fallback on relay failure, local
  mode `machine` and `messaging_supported: false`, local mode clamping `since`
  to `7d` with `since_capped`, a relay view carrying `console_owner` passing
  through, and text and JSON golden output.
