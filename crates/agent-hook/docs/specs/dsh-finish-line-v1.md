# DSH finish-line operations

Part of the [agent-hook v1 contract](agent-hook-v1.md#native-dsh-finish-line);
read its shared request boundary and public command inventory first.

The request and response contracts are:

- `open` adds a caller-generated unpredictable `attempt_token`. It
  deterministically derives and returns one unpredictable `runner_capability`
  bound to that token, the exact repository, and the DSH session, while storing
  only the capability digest and a 24-hour lease expiry. Retrying the exact
  binding returns `status = "duplicate"`, the same capability, and a renewed
  lease, so a lost success response is recoverable. A different attempt for a
  live session returns `finish-line-session-active` and cannot rotate or learn
  its capability. The capability derivation includes a persisted monotonic
  incarnation sequence. After authenticated release, a new attempt may open a new
  capability incarnation for the same stable session identity, including an
  rc.7 resumed session.

  The old capability remains tombstoned and cannot affect
  the new incarnation. The attempt token is private idempotency material for
  the current live incarnation, not an authorization bearer or permanent
  revocation identity. Reusing it after release therefore starts a new
  persisted incarnation and derives a byte-distinct capability; it cannot
  resurrect the retired bearer. Bounded tombstone compaction may age out the
  old release's duplicate receipt but does not roll back the monotonic sequence
  or make the old capability valid. The
  caller must keep both attempt token and bearer
  private; `run` rejects a missing, stale, or cross-session value.

  Linux clients may additionally send `owner_pid`, a positive integer naming
  their live same-user ancestor process. Null, non-integer, unrelated, or
  unverifiable identities are rejected with `finish-line-owner-invalid`.
  Nils derives the PID start time, boot identity, PID namespace, and UID from
  the kernel; request JSON cannot supply death or cleanup evidence. Exact
  retries must match both the token-derived capability and owner identity.
  A new verified owner can immediately resume the same session after nils
  proves the bound old process dead, stops every recorded contained unit, and
  verifies authoritative quiescence. Successful rotation returns
  `status = "recovered"` with a fresh capability incarnation.

  A live or unknown
  old owner, uncontained pending work, or unresolved mutation admission fails
  closed; lease expiry alone never authorizes takeover of an owner-bound
  session. Recovery invalidates the crashed session's validation evidence and
  claims before publishing new authority, terminalizes recoverable pending
  acceptance operations as infrastructure-blocked, and preserves any existing
  validation obligation. Completion requires fresh evidence. Late old run
  finalizers cannot claim replacement reservations or publish evidence.

  Owner bindings are stored in a bounded private `.owners` sidecar keyed by
  the main state's exact capability digest and incarnation. The strict released
  V1 main-state shape is unchanged. Interrupted sidecar/main writes preserve
  the committed binding; missing or mismatched bindings leave the session
  unbound and cannot authorize automatic recovery. Clients omitting
  `owner_pid` retain the existing unbound behavior and cannot claim this recovery guarantee.
  Containment-host validation precedes identity and state mutation; non-Linux
  activation fails with exit `69` and
  `finish-line-containment-unavailable`. Linux activation also requires the
  trusted fixed systemd binaries, unified cgroup v2, enabled unprivileged user
  namespaces, and a responsive user manager.

  After the physical cwd boundary is validated, an existing canonical directory
  with no worktree or bare Git boundary in itself or any ancestor returns exit
  `65` with the exact code `finish-line-not-in-repository`. A Bash operation
  may add its exact `command`; the same code is returned for any command,
  because outside every repository there is no validation contract to satisfy.
  A malformed command is `finish-line-command-invalid`. The consumer classifies
  that identity as obligation-free (ordinary shell, no
  edit generation, no stop boundary) while its ordinary policy transport keeps
  governing the command; `finish-line open` never authorizes execution by
  itself. Explicit `null` or any non-string command is request-invalid. A
  present but malformed or otherwise unprovable Git boundary remains
  `finish-line-repository-invalid`; consumers must branch only on the exact
  code and never on message text.
- `begin` adds `operation_id`, a caller-generated unpredictable
  `attempt_token`, and `operation = {"kind":"edit"}`. It atomically advances
  the repository's monotonic integer generation before the edit proceeds. The
  token is never echoed or persisted; only its digest is stored. Retrying the
  exact request with the same binding returns `status = "duplicate"`, so a lost
  success response is recoverable. A reused operation ID with a different
  binding is invalid.
- `run` adds `operation_id`, `intent`, the exact command string, the private
  `runner_capability`, an optional `execution`, and a timeout from 1 ms through
  60 minutes (30 minutes by default). The intent and command are compared
  byte-for-byte with the current repository validation contracts. The hook
  reads only the root `VALIDATION.toml`; a repository without it declares
  no validation contracts. Omitting `execution`
  performs the required non-executing capability and
  target probe: an exact current validation target returns `status = "ready"`
  with its target and contract digests, while a non-target returns
  `status = "ordinary-ready"`. Neither probe runs the command or creates
  execution evidence.

  With `execution.kind = "bash-v1"`, nils is the executor for both branches. An
  exact target is reserved before launch and may create validation evidence. A
  non-target first advances the shared repository generation, is registered as
  an ordinary shell operation, and then runs; the first terminal response has
  `status = "ordinary-applied"` even when its observed exit is nonzero. An
  ordinary operation never creates target validation evidence, and its
  generation advance makes prior validation evidence stale. Exact retries of a
  terminal exact or ordinary binding return `duplicate` without re-execution or
  output replay.

  Exact validation execution requires the authoritative repository root as its
  work directory. Ordinary execution preserves its canonical requested work
  directory while advancing the generation of the authoritative repository
  named by the common request identity. Each output stream is bounded to at
  most 64 KiB. `unsandboxed` and
  `danger-full-access` runners use the trusted fixed
  `/bin/bash -c <exact-command>` path. A `confined` runner executes the
  provider-supplied bounded argv only when it ends in the exact
  `-- bash -c <exact-command>` tuple; its declared mode, enforcement strength,
  denial signatures, and runner-failure rules are validated and returned as
  bounded sandbox facts. Git environment overrides are removed and stdin is
  closed. Unconfined runners retain the user service manager's supplementary
  groups. They do not promise the calling login process's inventory: an SSH
  login and an already-running user manager can inherit different groups.
  The preservation baseline is an independent transient user service; a
  caller/manager mismatch is diagnosed separately from a runner group change.

  Authoritative execution is Linux-only and fails closed elsewhere. On Linux,
  nils verifies the fixed `/usr/bin/systemd-run` and `/usr/bin/systemctl`
  executables as root-owned, executable, non-symlink regular files without
  group/world write bits. Before launch, nils serializes the bounded config into
  an anonymous memfd, sets mode `0400`, and requires the write, grow, shrink,
  and further-seal locks. No path-backed runner config exists.

  Systemd `OpenFile` opens `/proc/<supervisor-pid>/exe` as named descriptor 3,
  the sealed config memfd as named descriptor 4, and an unlinked mode-`0600`
  control memfd as named descriptor 5 for the unit. Nils reads the current
  runner's ELF program headers, verifies its dynamic interpreter as a root-owned
  executable without group/world write bits, and invokes that interpreter with
  `/proc/self/fd/3`. The contained runner accepts exactly the three expected
  systemd listen descriptors, verifies the config is an unlinked sealed regular
  file, and then parses the strict config. This binds execution to the current
  runner inode and immutable config rather than re-opening mutable path names.

  The config carries a random control nonce. Before spawning the provider, the
  contained runner validates descriptor 5, sets `FD_CLOEXEC`, makes its process
  non-dumpable, and publishes a nonce-bound `ready` record. The supervisor
  acknowledges only after that proves systemd opened the descriptors: it closes
  the config, changes control mode to `0400`, drops its writable control view,
  makes itself non-dumpable, and publishes the matching acknowledgement. The
  runner requires that exact acknowledgement and mode, then clears the
  acknowledgement before provider spawn. Provider code therefore cannot reopen
  a nonce-bearing config or writable control memfd through the outer or runner
  procfs descriptors; a forced stop before terminal publication leaves the
  intended empty, unsealed control state.

  After observing the child, the runner writes exactly one bounded strict-schema,
  nonce-bound provider exit, provider signal, or infrastructure-failure result
  and immediately seals descriptor 5 against all further writes. After the
  transient unit and cgroup are quiescent, the supervisor accepts only an
  already sealed result with the exact schema and nonce. Internal runner state
  therefore does not reserve provider exit codes or signals: exits 134 and 204
  remain exits, and Linux signals are returned as canonical `NodeJS.Signals`.
  An unrecognized signal fails closed.

  The config also binds the supervisor PID. The contained runner obtains a
  pidfd for that process and monitors it while the workload runs; supervisor
  disappearance triggers immediate workload process-group termination and a
  runner failure. Nils launches the runner in a transient user unit with
  `PrivateUsers=yes`, `Delegate=no`, `KillMode=control-group`, immediate
  `SIGKILL` stop/final-kill behavior, and a bounded `RuntimeMaxSec`. The unit
  admits only `AF_INET` and `AF_INET6` and denies localhost. Timeout or
  cancellation stops the whole unit. After the runner exits, nils queries the
  unit through the trusted `systemctl` and records execution facts only when
  `ActiveState` is `inactive` or `failed`.

  The transient cgroup boundary covers descendants that change process groups,
  call `setsid`, or double-fork. Private user-namespace capability reduction,
  the lack of `AF_UNIX`, and localhost denial close the tested parent-cgroup and
  user-manager delegation routes. These settings are not a general network
  namespace and do not claim to prevent every possible network or IPC
  delegation mechanism.

  Nils derives success or failure solely from the observed exit status, signal,
  timeout, and cancellation state. The caller cannot report an outcome,
  stdout, stderr, or completion decision. Initial execution returns the bounded
  observed output and execution facts. For an exact target, a later edit,
  changed contract, or newer attempt returns `stale` or `superseded` and cannot
  satisfy the current generation. A provider-runner or control failure does not
  become validation evidence and retains the pending unit binding until
  authenticated quiescence proves cleanup.
- `stop` resolves every exact command of every current DSH validation contract.
  It returns `action = "allow"` and exit `0` only when the current session has
  success evidence for every target at the current repository generation and
  exact contract digest. Missing, pending, failed, stale, or drifted evidence
  returns `action = "block"`, bounded reason and remediation arrays, and exit
  `1` while retaining `ok = true` in the service envelope. A repository at
  generation zero remains allowed. Once any session advances the shared
  repository generation, every session must supply its own success evidence at
  that current generation; an otherwise untouched session blocks rather than
  borrowing another session's evidence.
- `status` returns only the generation, current contract digest, correlation,
  target intent/digest/status, and optional attempt generation. It never returns
  raw commands, paths, identities, capabilities, tokens, or child output.
- The internal `release` request carries the common identity and exact
  session-bound `runner_capability`. It refuses release while any operation for
  that session is nonterminal or retains an active unit, including operations
  retained in the acceptance sidecar. Once quiescent, it removes the session
  and terminal operation records from the main execution state. Terminal
  acceptance evidence remains in the sidecar for deterministic recovery and
  verdict reconstruction. A bounded capability-digest tombstone makes an exact
  lost-response retry return `status = "duplicate"`. The tombstone binds that
  released capability incarnation rather than permanently retiring the stable
  session identity.

  A later open may create a new incarnation; retrying the old release remains
  duplicate while its bounded tombstone remains and cannot remove the new one.
  Reusing an old attempt token after release creates a new sequence-bound
  capability rather than resurrecting the old one. A defensive live-state
  check takes precedence over the duplicate path if persisted state ever
  contains both the tombstone and its matching live capability.

The internal `quiesce` request carries the common identity, exact
`operation_id`, and session-bound `runner_capability`. Pending validation and
ordinary operation records retain their generated `active_unit`. Cleanup
validates that binding and performs bounded trusted-`systemctl` stop and
inspection loops. Every loop also runs an exact-unit `list-jobs` barrier. A
single absent-unit observation is insufficient: cleanup requires three
consecutive observations 25 ms apart with no pending job and either a missing
unit or an inactive/failed, dead/failed unit whose extant cgroup reports
`populated 0`. Only after that stabilized proof does it remove the pending
operation and target attempt. For an exact contained-Bash acceptance source,
the same proof first records a durable `infrastructure-blocked` acceptance
terminal.

Execution/control errors retain `active_unit` and pending state so
authenticated cleanup remains possible. The idempotent result is
`status = "quiescent"`; it cannot create successful execution or validation
evidence, and the caller cannot choose the infrastructure terminal result.

There is no public `complete`, `waive`, `approve`, or `revoke` finish-line operation.
There is no caller-reported result path, review authority, waiver artifact, or
ambient waiver environment variable. Only a `run` execution observed by nils
can create success evidence.

The engine holds one transaction lock per repository for every state read or
mutation. Repository generation is shared across sessions, while target facts
remain product/session isolated. State keys bind the canonical
repository top-level and Git common-directory device/inode identities, exact
contract set, contract context, and exact command target. State never uses file
mtime as authority. The persistent repository lock is also an initialization
anchor: once created, disappearance of the paired state record fails closed
instead of resetting the generation. A safe prior marker is reported only as unresolved
transitional evidence; an unsafe marker is reported separately and neither can
satisfy native generation state.

Terminal operations are compacted deterministically by ascending sequence at a
256-record trigger down to a 192-record recent-idempotency window. Pending
operations are never compacted. Obsolete-generation target facts and empty
sessions without a runner capability are removed without changing current
generation enforcement. Authenticated release tombstones retain only digested
session and capability bindings plus sequence, are keyed by capability
incarnation, and compact to the most recent 64 entries; they do not consume the
live 64-session limit. Attempt tokens are not persisted. The monotonic
incarnation sequence makes every post-release capability byte-distinct even
when the caller reuses an old attempt token. Exact live `open` replay renews a
capability lease.

Lease expiry alone
never removes a session, terminal evidence, or pending state. At the hard
live-session bound, a bounded recovery scan may reclaim an expired crash orphan
or quiescent expired session. It retires only the one entry required for the
new admission. A persisted deterministic cursor rotates the eight-candidate
window across eligible sessions, so an older busy window cannot indefinitely
hide a later reclaimable candidate. Every busy operation must retain an exact valid transient-unit identity,
passive status shows no live unit or pending job, and bounded stop/status,
`list-jobs`, and cgroup checks then prove stable quiescence. The store is locked
again and the exact capability digest, lease, operation keys, sequences, and
unit identities must still match before the session is tombstoned and removed.

Active, indeterminate, unbound, oversized, or migrated no-lease sessions remain
protected. A reclaimed stable identity may open a new lease with fresh or
reused caller-held retry material. Every post-release open advances the
persisted incarnation sequence and therefore derives a byte-distinct
capability; the retired bearer remains invalid after its duplicate-release
tombstone is compacted. An excess of nonterminal
operations still fails at the hard
512-operation bound. Both `open` session admission and `begin` edit admission
may invoke this recovery, so a new DSH session does not need to execute Bash
before its first edit can reclaim provably quiescent orphan capacity.

Finish-line directories must be private and owner-controlled. Lock and state
files are opened with `O_NOFOLLOW`, checked through their open descriptors as
owner-controlled regular files, and bounded to 384 KiB. Each update writes a
private create-new temp file, calls `fsync`, atomically renames it, and requires
the parent-directory `fsync` to succeed before reporting success. Lock/open
unavailability exits `69`; bounded lock contention exits `75`. Pending
operations may persist the generated active-unit identifier needed by internal
cleanup. Raw validation output is returned only in the initial bounded `run`
response and is never persisted. Contained config and nonce-bound control state
are held only in anonymous memfds; the trusted runner seals their contents
before either is accepted. No Python handler, shell command rewrite, `EXIT` trap,
caller-reported outcome, or waiver state participates in the engine.

Policy validation also checks the complete provider/event/capability binding,
not only each component in isolation. `decision.warn.v1` and
`decision.context.v1` require an event with native model-context semantics;
`decision.block.v1` requires native block, continuation, feedback, or decline
semantics; and `decision.transform.v1` is limited to Codex `PreToolUse` plus
Claude `PreToolUse`, `PermissionRequest`, and `PostToolUse`. DSH does not
transform arguments; it accepts native allow/block and context capabilities at
the three supported boundaries.

`agent-session.owner-liveness.v1` and
`agent-session.semantic-conflict.v1` require both context and block semantics
because their result is data-dependent. Neutral `decision.allow.v1`, metadata
side effects through `agent-session.activity.v1`, and trusted provider-native
`runtime-kit.handler.v1` remain valid for supported Codex/Claude events. DSH
rules reject retired file handlers so policy cannot fall back to the archived
runtime-kit implementation. This preserves
notification and failure logging without pretending that events such as
Claude `Notification` or `StopFailure` can enforce a decision.

`agent-session.coordination.v1` is limited to enforceable
`PreToolUse`, `PostToolUse`, `PostToolUseFailure`, and `Stop` events and must be
an `enforce`, fail-closed, locked rule.
