# Provider setup, activity and recovery

Part of the [agent-hook v1 contract](agent-hook-v1.md).

## Activity helper resolution

`AGENT_SESSION_BIN` outranks `PATH`, so a stale value inherited from a
long-lived tmux server survived the relocation the `PATH` pin was added to
prevent. An override that does not resolve to an executable regular file is
therefore treated as absent, and the daemon-pinned `PATH` decides instead. This is
a deliberate change to a fail-closed boundary rather than a downgrade: anyone able
to set `AGENT_SESSION_BIN` can already set `PATH`, and the pinned `PATH` is
daemon-controlled. The resolved executable is pinned as an absolute path for
the activity attempt. The discarded override is recorded as
`activity-helper-unresolvable` so it cannot silently mask a misconfiguration, and
an empty assignment is the normalized "resolve through the pinned `PATH`" value.

Ingress registration is not capability health. When the selected policy binds
`agent-session.activity.v1`, `doctor` resolves the helper to an executable file
through the override or `PATH` before reporting an otherwise converged provider as
healthy, and fails with `activity-helper-unresolvable` when it cannot.

Rule modes are `enforce`, `advise`, `shadow`, and `disabled`, with authority
3, 2, 1, 0. `advise` evaluates exactly like `enforce`, with the same side
effects and child budget, but projects a `Block` outcome to `Context` carrying
the same code, the rule's remediation (a Tier B group's governed replacement,
or the shell retry guidance), and one final line `downgraded to advise by
<config path> [overrides.<rule id>]`. Allow and Context outcomes are unchanged.
Shadow evaluation records only a redacted observation: it cannot affect exit
status/output authority, rewrite input, mutate capability/rule/session state,
perform reclaim/adoption, or consume recovery. Policy order and results are
deterministic across providers for overlapping capabilities.

`dsh.policy.v1` groups carry one of three enforcement tiers, frozen with the
capability-group fixture and reported per rule by `inventory` as `tier` and
`enforcement_default`:

- `integrity` (Tier A: `owner-unclaimed`, `semantic-conflict`,
  `operation-lifecycle`, `agent-scope-lock-guard`, `checkout-lease-guard`,
  `mcp-secret-scan`, `finish-line-record`) blocks and must be declared
  `enforce`, `closed`, and `locked`; anything else fails with
  `tier-a-rule-not-locked` (or the older `coordination-rule-not-locked` for the
  operation lifecycle).
- `governed-seam` (Tier B: `block-direct-git-commit`,
  `block-direct-git-worktree`, `block-direct-pr-create`,
  `block-unsafe-default-delivery`, `semantic-commit-body-gate`,
  `block-project-memory-write`, `portable-paths-scan`, `pre-edit-intent-gate`)
  blocks by default and must be `closed` and `locked` or `downgrade-only`; a
  `free` declaration fails with `tier-b-rule-not-downgradable`. Every Tier B
  denial carries remediation naming the governed replacement. A config
  override to `advise` is the only downgrade runtime-kit writes.
- `reminder` (Tier C: `agent-activity`, `block-direct-python`,
  `forge-label-reminder`, `memory-write-principle-reminder`,
  `session-start-healthcheck`, `skill-usage-reminder`,
  `stop-pre-pr-reminder`, `user-prompt-agent-memory`) yields context only,
  must be `locked` (`tier-c-rule-not-locked`), and its group evaluator never
  returns a block (asserted in debug builds; the identity-incomplete activity
  failure in the dispatcher is not a group outcome). `block-direct-python` is
  therefore advisory: under a uv or venv project it names the manager and the
  expected interpreter instead of denying. `user-prompt-agent-memory` also
  carries the coordination mailbox reminder on every DSH model step.

  For an
  authenticated managed session (`AGENT_SESSION_ID`, plus an absolute
  `AGENT_SESSION_CAPABILITY_FILE` and `AGENT_SESSION_STATE_DIR`), it runs the
  same-release `agent-session message reminder`, which claims each live
  unread generation once. It appends the reminder only when the text is the
  fixed body-free prompt for that session. A refusal, timeout, or any other
  output adds nothing, so the step is never blocked.

A rule declared `mode = "advise"` and a config override to `advise` are both
rejected with `rule-override-advise-unsupported` when one of the rule's product
events cannot render context, because the projection would otherwise be a
silent allow. The `downgraded to advise by` line and `downgraded_by` render the
config path with the home directory folded to `$HOME`.

Consumer migration (dsh-runtime-kit): an unclassifiable Bash command now
blocks only Tier A groups and the Tier B groups whose subject the command
names, its non-blocking explanation is carried as context rather than as a
denial, and `block-direct-python` never appears as a blocking reason. A
consumer heuristic that expected every command-dependent group to block at
once (runtime-kit's opaque-shell fan-out fallback) can be retired; the
decision `context` already carries nils' own guidance.

A DSH `Context` outcome is emitted once per session: when the same
`(session id, rule id, context digest)` was already emitted, the outcome
becomes an allow whose code carries the `:advisory-repeated` suffix. The
markers live under the subject's agent-docs state home in
`agent-hook/dsh-advisory-dedupe/<sha256(session)>/`; when that directory cannot
be created privately the reminder keeps rendering (fail open, reminders only).

Ordinary selected rules are evaluated and aggregated before the typed session
coordination phase. A blocking `PreToolUse` aggregate returns without invoking
coordination; an allowing aggregate invokes the exact #676 consumer and merges
its typed result without weakening the earlier decision. Terminal
`PostToolUse`, `PostToolUseFailure`, and `Stop` deliveries invoke the consumer
after ordinary aggregation even when that aggregate blocks, because an already
admitted operation must persist its outcome and complete or remain explicitly
reconcile-pending. Shadow evaluation never invokes the consumer.

Override classes:

- `locked`: user config cannot change mode, priority, posture, or parameters.
- `downgrade-only`: config may move `enforce -> advise -> shadow -> disabled`,
  never in the other direction and never change typed parameters.
- `free`: config may choose any mode but still cannot add a command/capability
  or change the policy rule identity.

## Setup ownership and rollback

`agent-hook setup --product <provider>` is dry-run-first. `--apply`, `--repair`,
and `--remove` are mutually exclusive. Mutation requires an exact reviewed
plan digest when drift, compatibility, or dual ownership is present. Setup
parses and renders all files before mutation, locks by a stable provider-path
identity independent of `--state-dir`, re-reads every reviewed byte state,
writes atomically, and restores all prior bytes and file-presence states on any
partial or post-replacement failure. Symlinks, non-regular files, unsafe
permissions, concurrent drift, and malformed provider roots fail without
mutation. JSON provider configuration is decoded with recursive duplicate-key
rejection before every plan, apply, repair, or remove path.

Setup resolves provider configuration roots from `CODEX_HOME` for Codex and
`CLAUDE_CONFIG_DIR` for Claude when the selected path is absolute. Relative
overrides are ignored. Without an absolute override, setup uses the provider's
default directory under `HOME`; Codex uses `$HOME/.codex` and Claude writes
`settings.json` under `$HOME/.claude`.

For Codex, `config.toml` and compatibility `hooks.json` are one transaction.
The managed `config.toml` representation also owns the authoritative
`agent-session activity notify --agent codex` argv: a safe singular user
notifier is composed by direct argv, and remove restores it byte-for-byte.
An exact Computer Use `turn-ended --previous-notify <JSON argv>` wrapper is
treated as composed only when its bounded chain reaches that exact owned argv
and the helper is the executable regular file at the fixed path below the
active Codex config root, with no symlink in any relative path component.

Alternating Computer Use/agent-session wrapper growth is drift: preview does
not authorize mutation, and an exact reviewed plan digest is required to
normalize it to one Computer Use wrapper plus one owned notifier. This
normalization is semantic rather than byte-reversible. Remove performs no
helper execution and may structurally strip exact owned layers at the fixed
path even when the helper is missing or no longer executable; unknown shapes
retain the ordinary foreign-notifier rules.
Prior `agent-runtime-kit:hooks` marker lines are validated as an exact ordered
pair outside TOML multiline values.

When Codex has saved trailing `[projects]`
or `[hooks.state]` trust tables inside that marker pair, setup classifies the
boundary as drift and an exact reviewed plan digest may move only the closing
marker ahead of the byte-preserved trust-table suffix. Ambiguous marker layouts,
noncanonical trust headers that cannot be moved byte-for-byte, and non-trust
TOML following that suffix fail closed before mutation.

The `agent-hook:provider-ingress:v1` ownership markers are recognized only as
an exact ordered pair of standalone lines outside basic and literal multiline
TOML values. Setup validates the complete foreign-manager marker layout before
the first-install no-owned path or any owned-span overlap classification; each
foreign owner may identify exactly one balanced range. An owned block wholly
inside a valid foreign-manager range is drifted and requires the exact reviewed
plan digest before it is moved outside that range. A foreign range inside or
partially crossing the owned span cannot be preserved by regeneration and fails
closed, as do orphaned, reversed, crossed, partial, or duplicate foreign markers
anywhere in the document.

Because the owned block is rendered last, Codex saves new tables such as
`[hooks.state.*]` trust records, `[marketplaces.*]`, and `[plugins.*]` inside
the markers. Only the prefix before the first table outside `[[hooks.<event>]]`
(with its directly preceding comments) is compared with the rendered block.
The appended suffix is drift that needs the exact reviewed plan digest. Apply,
repair, and remove move that suffix byte-for-byte ahead of the regenerated
block, or to the block's place on removal, and never drop it. Owned hook tables
after that suffix cannot be moved without reordering hook arrays, so setup fails
closed.

Owned groups contain exactly one dispatcher command for each required
event/matcher. Install, upgrade, repair, remove, and rollback preserve
unrelated hooks, comments, formatting, provider metadata, and unsupported
surface truth. A no-op remove preserves both bytes and file presence; removal
deletes only an exact owned representation. The `--remove --dry-run` setup
combination is the removal-specific no-write preview. It returns
`action: "remove-dry-run"`, `changed: false`, `would_change`, `apply_allowed`,
and the reviewed `plan_digest`; `--remove --expected-plan-digest <digest>`
accepts that digest only while the exact before/after bytes and file-presence
plan remains current. Pre-dispatch
`agent-session activity setup` forwards to this API and cannot install a second
managed representation. Compatibility handlers are reported as compatibility-only or `dual`
until the reviewed migration removes them.

The exact historical `session-coordination-guard.py` command at its fixed
60-second timeout is recognized as compatibility state only when the selected
policy contains the typed coordination capability. The reviewed setup plan
removes that sibling handler and renders one 60-second `agent-hook dispatch`
entry in each required event/matcher group; arbitrary lookalikes remain
unrelated.

## Governed recovery

Recovery state paths are validated without following symlinks before any
chmod, creation, or write. A challenge loads the current strict policy and
binds the exact product, event, target digest, command
digest, snapshot digest, requested rules, scope, state revision, and expiry.
Authorization requires the same effective OS user, a private regular challenge
file, explicit digest confirmation, and a fresh state revision. It produces a
private capability file and a redacted audit record.

One-shot scope is consumed atomically once. Repair-window scope binds one
session principal, explicit target set, permitted locked rule IDs, a maximum
15-minute duration, and a monotonic state revision. It can be revoked early.
Capabilities fail closed on absence, ambiguity, expiry, replay, revocation,
permission/owner mismatch, product/event/session/target/command/snapshot drift,
target recreation, or key rotation. No environment variable or persistent
config enables bypass.

An exact capability may recover from malformed config or missing policy by
evaluating its signed, versioned rule manifest. Unknown/non-recoverable IDs are
rejected before authorization, and ungranted rules retain their action or
failure posture. It
does not bypass OS/provider authorization, unrelated hooks, nils-cli
transaction/privacy invariants, or a scope not explicitly bound in the
capability. Traces and output expose only a reason code and capability digest,
never the bearer or private identity.

The typed session coordination rule is present in the signed emergency
manifest but is never a recoverable rule ID. Config-independent recovery still
invokes it after the recovered aggregate allows, so break-glass cannot bypass
the #676 transaction.
