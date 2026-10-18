# DSH workspace leases and recovery

Part of the [agent-hook v1 contract](agent-hook-v1.md).

## Native DSH workspace lease

`agent-hook workspace-lease <command> --format json` reads one strict JSON
object from standard input, capped at 256 KiB. Duplicate keys, unknown fields,
unsupported versions, non-absolute supplied cwd values, invalid lifecycle
enums, and oversized or control-bearing identity strings fail with exit `65`.
The public surface is exactly `resolve`, `bind`, `begin`, `complete`,
`renew`, and `release`; every command defaults to its versioned service JSON
envelope. `resolve` exists only in protocol v2.

`bind` accepts the runtime-owned WorkspaceLease v1 facts: request, session,
optional parent, optional cwd, and DSH session-start source. The provider
canonicalizes cwd without inherited Git repository-selection state. A managed
identity binds the physical Git top-level and its device/inode identity, the
per-worktree Git directory, and the shared Git directory. Thus nested and
symlink-equivalent path spellings converge while two linked worktrees remain
distinct. A missing or non-Git cwd produces an `unmanaged` binding and never a
mutation lease.

Managed state permits one active binding generation per physical worktree. A
clean unowned checkout becomes `owned`; a live other owner returns
`foreign-active`; dirty state returns `dirty`; and an expired binding with an
active operation returns `uncertain`. An expired or explicitly released clean
binding with no active operation may be replaced atomically. Replacement mints
a new binding ID and generation and tombstones the prior generation so delayed
release cannot affect the new owner. No expired exact bind replay can revive
old authority.

An explicit `resume` or `compact` may also replace a dirty released or expired
generation when the host-authenticated session and optional parent lineage are
exactly the same and every operation is terminal. This is recovery of the same
owner's work, not reassignment: it still mints a new binding ID and generation.
A startup, clear, foreign session, changed parent, or unterminated operation
does not qualify for same-principal recovery.

Protocol v2 also permits a different session to bind an explicitly resolved
target after its former owner released the binding with every operation
terminal. The target remains dirty and its bytes are preserved; replacement
atomically fences the former generation. A dirty checkout without a released
binding, a live foreign owner, an expired owner that never released, or an
unterminated operation remains unavailable for cross-session handoff. The
session's starting cwd does not select or authorize this v2 target.

`begin` binds the exact call/root/tool/arguments/nesting facts to one operation
ID and unpredictable fence before tool dispatch. Known read-only DSH tools
return `not-required`; unknown tools are conservatively fenced. `complete`
requires the exact live binding, generation, operation ID, fence, and
call/root/tool projection, and records one of `succeeded`, `failed`, or
`cancelled`. `renew` extends only a still-live exact generation. `release`
retires only its exact generation and fails unavailable while any operation
lacks a terminal outcome.

Active bind/begin retries and terminal complete/release retries are idempotent.
Reusing a request ID for different facts fails closed. State is stored below
the private `workspace-leases/` state root using bounded owner-only files,
atomic replacement, and a bounded per-workspace cross-process lock. A clean
expired owner or the exact explicit same-session recovery above can be
recovered; foreign dirty state and active-operation, malformed, identity-
mismatched, untrusted, busy, or unknown state are never guessed through.

### Protocol v2: target-scoped repository authority

Protocol v1 binds one immutable session cwd, so one dirty checkout denies every
later operation in that session. Protocol v2 keeps the durable per-worktree
authority model unchanged and moves selection from the session anchor to the
exact operation target.

`resolve` classifies one exact tool execution. It writes no lease or binding
state and never binds, fences, or denies a lease, but it is not side-effect
free: like every operation needing the keyed workspace digest it mints the host
`fingerprint.key` on first use, and it fails closed with
`workspace-target-unresolvable` when the declared target arguments are
unusable. It accepts the trusted anchor cwd, the exact
call/root/tool/arguments/nesting facts, and returns either `not-required` or a
`targets` array.

A tool is classified when its own declared arguments prove the repository it
mutates. That is the axis, not whether the tool is a shell:

- `write` and `edit` prove it through `file_path`, and `str_replace_editor`
  through `path` for `create`, `str_replace`, and `insert`.
- `artifact_export` proves it through `destination.path` when
  `destination.class` is `workspace`, a path its own schema constrains to be
  workspace-relative. The `download` class writes nothing into a repository
  and proves no target.
- `runtime_kit_governed_commit` declares no path because it has no target to
  choose: it always commits the canonical live session workspace. The trusted
  anchor is therefore the whole proof, and without an anchor there is no
  admissible target rather than an unscoped commit.

Read-only forms, unknown tools, and arbitrary shell programs resolve to
`not-required` and run as unscoped native host operations, because their
repository effects cannot be proven by inspecting a startup directory. For a
shell that is deliberate rather than incidental: a `workdir` fence is defeated
by `cd` inside the command string, so honouring one would claim coverage this
boundary cannot enforce. This removes the v1 fence -- cross-session
exclusivity, the dirty gate, and the uncertain-outcome gate -- from shell
mutations, which is an accepted trade-off recorded in
`sympoies/dsh-runtime-kit#172`; whole-runtime isolation, not a path argument,
is the boundary that can bound a shell. A non-repository target is omitted, so
a write outside any checkout needs no repository lease.

Each classified path is resolved against the trusted anchor when relative,
canonicalized through the same mutation-target binding used by the policy
boundary (symlinks resolved, non-existent leaves bound to their nearest
existing ancestor, inherited Git repository-selection state rejected), and
reduced to its physical Git top-level. Targets are deduplicated and returned
sorted by their keyed workspace digest. Every tool classified today yields at
most one target, so that ordering is the wire contract for a future multi-path
tool rather than a rule exercised now: when one arrives, a deterministic
acquisition order is what keeps two sessions from deadlocking against each
other. A relative path with no anchor fails closed rather than guessing a
repository.

A v2 `bind` accepts either an exact resolved `target` or an anchor `cwd`,
never both. A target bind rederives canonical identity from the live host
layout and requires the keyed workspace digest to match, so a forged root, a
subdirectory, or a drifted checkout cannot select another workspace's
authority. An anchor bind is an optional eager optimization only: a
non-repository or absent anchor returns `not-required` instead of minting an
unmanaged binding, and its denial is local to that one repository. Every other
v2 binding decision -- `foreign-active`, `dirty`, `uncertain`, expiry,
tombstoning, and same-session `resume`/`compact` recovery -- is the unchanged
v1 rule applied per physical worktree. One session therefore owns independent
bindings for repositories A and B, while two sessions targeting one physical
worktree still contend.

An exact-target v2 bind may set `takeover_capability: true`. Only opted-in
foreign-active denials include an optional `conflict` digest. A later bind with
that digest in `takeover_conflict` rechecks the current binding, branch and
HEAD under the workspace lock, rejects an active operation, and replaces the
former generation. The conflict is a compare-and-swap value, not an approval
receipt; the DSH caller obtains one tool approval before sending the transfer.
Older v2 callers omit the capability and receive the unchanged strict denial
shape. An unborn branch uses its symbolic HEAD name and an explicit unborn
marker; creating the first commit invalidates that conflict.

The opted-in retry returns `WORKSPACE_TAKEOVER_STALE` if the conflict changes,
and `WORKSPACE_OPERATION_UNCERTAIN` while the former owner has an active
operation. The current DSH runtime passes
`DSH_RUNTIME_KIT_WORKSPACE_LEASE_V2=1` only while its native provider is
registered, which makes the older cwd-scoped checkout policy defer to that
exact-target lease; older runtimes continue to use the earlier policy.

Every v2 `bound` result names the exact repository target it owns, including
its `workspace_key`. Convergence between an eager anchor binding and a later
lazy acquisition of the same repository is the runtime's obligation, not a
boundary guarantee: the runtime must key its authority set by that
`workspace_key` and reuse the existing binding for any resolved target with the
same key. A second bind of a workspace this session already owns is denied
`foreign-active` like any other contention, and that denial is
indistinguishable from a genuinely foreign holder, so a runtime that eagerly
binds its anchor and then binds again per resolved target would contend with
itself.

A v2 `begin` additionally carries the exact resolved target and proves it is
the same workspace as the durable binding. Classification already happened in
`resolve`, so `begin` never reclassifies a tool name and every admitted v2
operation carries a fence.

That fence is on the workspace this call was classified to, not merely the one
the caller names. `begin` proves both: `require_target_identity` authenticates
`target` against the durable binding, and the `target_token` the request
carries beside the target proves `resolve` produced that target for these exact
call facts. `resolve` mints one token per returned target, keyed over the host
fingerprint, the target's workspace key, and a digest of `call_id`,
`root_call_id`, `tool_name`, `arguments`, `anchor_cwd`, and `nested`. `begin`
recomputes it and rejects a mismatch with `workspace-target-call-mismatch`,
which stays distinguishable from the `workspace-target-invalid` identity
verdict.

The anchor is part of the binding because some targets are derived entirely
from it: a governed commit names no path, and a relative `file_path` resolves
against the anchor. One set of call facts therefore classifies to a different
repository under a different anchor, so a token that omitted it would
authenticate either target for the same call. A v2 `begin` carries the same
`anchor_cwd` it gave `resolve`, and a v1 `begin` rejects both the token and the
anchor as v2-only fields. The `arguments` digest is taken over a key-ordered
projection, so an adapter that reserializes them between its `resolve` and its
`begin` is not denied for a difference that carries no meaning. A caller
that names a repository it legitimately holds while its arguments mutate a
different one therefore fails closed instead of being fenced on the wrong
workspace, and the token cannot be forged without the host fingerprint key.

`begin` still never derives the target from the tool name and arguments itself,
so the classifier stays in one place.

Two consequences follow. A call `resolve` classified as `not-required` has no
token, so a `begin` that names a target for it fails closed rather than fencing
an operation the classifier already excused. And `bind` is unaffected: it
authenticates identity by rederiving the workspace digest from the live layout,
depends on no call facts, and accepts no token. `complete`, `renew`, and
`release` keep their v1 fields, fencing, idempotency, and terminal-outcome
rules.

A v2 `resolve` or target result carries the canonical repository root of the
caller's own exact operation targets. That is the one deliberate exception to
the projection rule below: it reveals nothing the trusted same-host caller did
not already supply, and the runtime-kit adapter must not project it into
model-facing tool output. Each resolved target's `token` is opaque keyed
material the runtime passes back through `begin` unmodified; the adapter must
not project it either, and must not reuse a token across calls.

Provider-visible results contain opaque IDs, renewal timing, and stable
`owned`, `unmanaged`, `foreign-active`, `stale-clean`, `dirty`, or `uncertain`
states with bounded stable codes/reasons. Results and diagnostics never expose
canonical paths, Git directories, session/parent IDs, tool arguments, command
output, or persisted digests. Canonical paths exist only in the private state
needed to revalidate physical identity and run a trusted, hooks-disabled Git
dirty-state probe.

## Native DSH workspace recovery

`agent-hook workspace-recovery inspect --format json` and
`agent-hook workspace-recovery verify-handoff --format json` each read exactly
one strict, duplicate-free JSON object from standard input. Requests are
limited to 64 KiB and carry protocol version `1`, an absolute current checkout
path, and, for handoff verification, one exact absolute candidate path.

The result projects only canonical checkout/worktree paths, branch and object
identity, bounded dirty path names with typed status states, managed/bare/
detached/prunable flags, and an optional verified handoff. It never includes
file contents, environment values, Git command output, lease authority, or
provider payloads. Result data is capped at 192 KiB so the complete service
envelope stays within the DSH client's 256 KiB transport cap. Oversized dirty
and worktree arrays are deterministically shortened and report
`dirty_entries_omitted` / `worktrees_omitted`; omission never changes the
dirty decision.

Inspection runs in-process through fresh libgit2 repository contexts. The
process registers no repository-provided filters and does not launch Git or
any repository command. Regular status excludes submodules from
repository-controlled ignore semantics; every submodule layer is instead
checked explicitly with `SubmoduleIgnore::None` through a bounded recursive
walk. Repository cycles, excessive depth/count, ambiguous paths, and
open/status failures fail closed. Index refresh is disabled and
workspace-lease records are never written. Recoverable exit `69` failures
carry typed `retryable`, `next_action`, and bounded `recovery` details.

A handoff is eligible only when it is a different, clean, live, non-bare,
non-detached, non-prunable worktree below the exact managed root convention
owned by `git-cli worktree`. Verification does not create, clean, stash,
commit, switch, adopt, or transfer the candidate. Its output is advisory to
the host: a fresh DSH session at that exact path must independently acquire a
normal workspace lease, so post-verification drift remains fail-closed.
