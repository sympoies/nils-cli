# Forge identity policy v1

Canonical shared contract for `forge-cli`, `git-cli`, `semantic-commit`, and the
`nils-common::git` runners. Phase 1 selects a configurable principal's identity,
refuses ambiguity or mismatch, and records private metadata. It does not provide
credential isolation from processes with the same operating-system privileges.

## Policy discovery and launch interface

The policy is `${XDG_CONFIG_HOME:-$HOME/.config}/forge-cli/identity.toml`.
When absent, existing behavior remains unchanged. An optional root-level
`activation = "asserted-only"` also leaves ordinary managed API, Git transport,
and commit behavior unchanged when `FORGE_IDENTITY_PRINCIPAL` is **absent**.
This applies to every provider and repository; no target selection, credential
probe, identity override, or identity audit runs in that case. `identity explain`
and `doctor` report `enforced: false`. An empty, invalid, or unknown asserted
principal still activates enforcement and refuses; an asserted principal has no
unmatched-target fallback. The default (omitted, or `activation = "always"`)
retains Phase 1 strict enforcement whenever the file exists. Policy parsing and
validation always run before the activation decision, so malformed policy still
refuses even with no principal assertion. This is a trusted-launcher rollout
choice, not isolation from processes with the same operating-system privileges.
Unreadable or malformed policy
refuses protected operations. Pure local Git inspection does not load credentials.

A launcher supplies `FORGE_IDENTITY_PRINCIPAL` as a stable configured principal ID
selected from the session's starting principal/role. Optional
`FORGE_IDENTITY_SESSION` supplies a metadata-only opaque launch reference for audit.
These variables contain no credentials. Phase 1 consumes this launcher assertion;
it does **not** authenticate an environment string or bind it to `agent-session`.
Phase 2 owns authenticated owner/role registration, inheritance, resume, and expiry.
No account, device, operating-system login, or principal role is inferred.

Principals can represent contributors, coordinators, reviewers, and other service
roles. They have explicitly listed profiles. Rules cannot assign profiles outside
that principal's list. No implicit principal or cross-principal delegation exists.

## Strict TOML schema

`version = 1` is required. Unknown fields, unsupported versions, malformed target
keys, missing references, duplicate rule IDs, and invalid selectors refuse.
Repository keys are lowercase `host/owner/repository`; GitLab keys may include
nested groups as `host/group/subgroup/project`. Organization keys are `host/owner`
for GitHub and `host/group[/subgroup...]` for GitLab. An organization key matches
only the immediate parent namespace of a project: `host/group` does not match a
project at `host/group/subgroup/project`; that project is matched by
`host/group/subgroup`. Hosts have no userinfo, port,
scheme, or path. The GitHub SSH hostname alias canonicalizes to the API hostname.
Lowercase self-hosted GitLab authorities must be listed in the root `gitlab_hosts` setting;
`gitlab.com` is recognized without configuration. An unknown self-hosted authority
reports that `gitlab_hosts` must be configured. Configuration contains metadata and
credential **reference names**, never tokens or private keys.

```toml
version = 1
gitlab_hosts = ["gitlab.example.invalid"]

[credentials.contributor]
kind = "gh_user"
user = "example-contributor"

[profiles.contributor]
expected_login = "example-contributor"
credential = "contributor"
commit_name = "Example Contributor"
commit_email = "contributor@example.invalid"
signing_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
operations = ["api_read", "api_write", "git_read", "git_push", "commit"]

[principals.contributor]
profiles = ["contributor"]
default_profile = "contributor"
default_repositories = ["github.com/example/default"]

[[rules]]
id = "contributor-project"
principal = "contributor"
repo = "github.com/example/project"
profile = "contributor"
```

The example is illustrative metadata; configure actual authorized credentials and
signers outside the repository. `gh_user` retrieves the named stored account with
`gh auth token --hostname HOST --user USER`, with inherited token overrides removed.
`kind = "env"` instead requires `name = "CONFIGURED_CREDENTIAL_REFERENCE"`; the
credential owner supplies that reference through the existing credential backend.
This feature does not change credential storage, mint tokens, or switch gh accounts.

A profile has exactly one actor form: `expected_login`, or `expected_app_id` plus
`app_slug`. An App profile verifies its public App ID, the credential's GraphQL
viewer as the corresponding bot, and installation-token coverage of the target
repository. An installation credential without proven coverage refuses. Human
credentials verify the authenticated `/user` login. Permission failures propagate;
no alternate profile or active-account fallback occurs.

`commit_name`, `commit_email`, `signing_fingerprint`, `credential`, and nonempty
`operations` are required on every profile. OpenPGP signing fingerprints contain
40 or 64 hexadecimal characters. Signing requires a usable local secret signing
key matching that exact fingerprint. API reads/writes and Git transport do not
require the signer; `doctor` checks both credential and signing readiness.

## Unasserted non-interactive runs

Optional root-level `unasserted_noninteractive = "allow" | "warn" | "deny"`
(default `warn`). It applies only to the `activation = "asserted-only"`
pass-through, where no principal is asserted.

- Non-interactive: stdin or stdout is not a TTY, or `INVOCATION_ID`,
  `GITHUB_ACTIONS` or `AGENT_SESSION_ID` is set. Interactive runs are never affected.
- `warn`: one stderr line per process names the fix (set `FORGE_IDENTITY_PRINCIPAL`,
  or set `unasserted_noninteractive = "allow"` to keep the shared default account
  on purpose), an audit record is appended, and the run continues. A failed audit
  append does not fail the run.
- `deny`: the audit record is appended and the run refuses with
  `identity_unasserted_noninteractive` (DATA, exit 65) carrying the same fix text.
- `allow`: silent.
- Audit volume: at most one record per caller (argv0 and parent) per hour, tracked
  by a stamp file under `forge-cli/unasserted-callers` in the state root; the
  warning itself prints once per process. The decision is local and adds no
  identity probe or provider call. The cap holds for sequential runs; parallel first
  runs of one caller can each record before the stamp exists, and under `deny` the
  generic refusal record is uncapped.
- The record's `outcome` is `unasserted_noninteractive_warned` or
  `unasserted_noninteractive_denied`. It holds only `caller.argv0` and
  `caller.parent` (base names), never environment values, tokens or paths.
- A run that asserts a principal, including the shared account's own, is unaffected,
  as are `identity explain`, `identity doctor`, `git-cli transport` and raw `gh` callers.

## Authenticated launch binding

`require_session_binding = true` opts managed calls into the authenticated launch
contract. Its default is false, preserving the Phase 1 assertion behavior. When
true it takes precedence over `activation = "asserted-only"`: removing the
principal assertion cannot disable the required binding. Missing policy still
preserves ordinary behavior.

Configure principal selection independently from repository/profile rules:

```toml
require_session_binding = true

[[launch_rules]]
id = "operator-default"
initiator = "operator"
principal = "contributor"

[[launch_rules]]
id = "operator-review"
initiator = "operator"
role = "reviewer"
principal = "reviewer"
```

This fragment belongs to a complete version 1 policy with the referenced
principals/profiles/credentials configured. Identifiers and accounts are
configuration, not built-in role mappings. A matching explicit role rule wins
over the initiator's rule without a role. A rule without a role is the explicit
default for that initiator. No match or multiple matches at the chosen precedence
refuse, even when they select the same principal.

An operator CLI launch supplies `agent-session start --forge-initiator ID` and an
optional `--role ROLE`. The immutable `lineage.forge_context` stores initiator and
role. Managed children authenticate their parent broker, inherit the initiator,
and record their explicitly assigned role; they cannot replace the initiator.
An authenticated managed `--no-parent` start preserves the initiator only when
the new lineage root carries a role; a role-less root started by a managed
session has no binding, so a bound role-less root always comes from a
non-session launcher. Roles are never inherited. Resume and adoption
preserve the original context.

An operator launcher may additionally request `--attest-role` with both flags
to record `attested_role` for that session. The broker projects this optional
field only from immutable launch context; it must equal the session's role.
Managed callers cannot request it, and child launches clear it even when the
child chooses the same ordinary role or starts a new lineage root. Existing
immutable bindings without the field remain valid and carry no attested role.
A consumer requiring operator-attested role authority must match this field;
existing identity launch-rule matching continues to use the ordinary launch
context and does not turn a role label into a privileged attestation.

Each resolver calls `agent-session broker identity --session ID --format json`
under the caller's probe deadline. The producer verifies the private session
capability, live heartbeat/registry and current runtime incarnation. The consumer
requires exact session/incarnation agreement with its managed environment.
`FORGE_IDENTITY_PRINCIPAL`, when present, is only an assertion and must equal the
mapped principal. Selection then uses all existing repository/profile/actor and
signer checks. Missing, stale or conflicting bindings refuse before credential
lookup. `FORGE_IDENTITY_AGENT_SESSION_BIN` selects the broker executable for
configured installations and fixtures.

`identity explain` includes the authenticated root/parent/current session,
initiator, role and matched launch rule under `selection.session_binding` without
reading forge credentials or writing an audit. Managed operation audits record
this launch provenance, the mapped principal and the asserted principal separately;
ambiguity retains the available launch context even without a selected principal.
Policy digest identifies the configuration used for each decision.

This is a same-operating-system-user trusted-launcher contract. It creates no
credential isolation or cross-principal delegation. Existing sessions without a
launch context must be relaunched before enabling required binding.

The runtime command owner must preserve the managed session ID, incarnation and
capability reference and assert through this projection. Generic HTTP session
creation refuses supplied forge launch context: authenticating an HTTP caller
does not verify its asserted initiator or role. Console roots and bound child
forwarding require a separately implemented, verified launch-owner protocol;
forwarding context to generic HTTP creation fails closed. These counterpart
changes are separate rollout work; this repository implements the local producer
and shared forge/git consumer only.

## Resolution

Resolve **within the starting principal**:

1. Exact repository rule.
2. Organization rule.
3. Managed-path rule.
4. Explicit principal default, only for `default_repositories`.

Each rule has `id`, `principal`, `profile`, and exactly one of `repo`, `org`, or
`path`. Path rules additionally require a nonempty `repositories` allowlist and
an existing absolute canonical filesystem path; symlink aliases and noncanonical
spellings refuse during policy validation. A linked worktree uses the source checkout identified
by its Git common directory, rather than the runtime worktree directory.

Multiple matching rules at the selected precedence refuse, including duplicates
that select the same profile. A matching path rule whose repository allowlist or
profile disagrees with a remote rule refuses. Repository rules deliberately
override organization profiles. Profile operation permissions apply after
selection; denial never falls through to a less-specific profile.

API targets come from explicit provider host/repository context and constructed
backend requests. A backend request with conflicting targets refuses. Opaque
GraphQL node mutations remain bound to their typed repository invocation.
Cross-repository reads (`inbox list/status/next` and `activity commits/events/summary`)
without `--repo` select the starting principal's sole distinct profile. Multiple
profiles refuse with `identity_target_ambiguous`, candidate profile IDs, and guidance
to pass `--repo owner/repo` or choose a principal with one profile. A repository
in the current checkout does not scope these reads. The selected profile must allow
`api_read`. The target host must be declared by a repository, organization, or path
allowlist rule for that principal and profile, or by its default repositories when
that profile is the default. These host declarations apply outside a checkout;
an undeclared host refuses with `identity_repository_unknown` before credential
lookup or actor probing. Credentials and actors are verified as usual. Host-only audit targets
omit `repo`, and App coverage checks apply only to concrete repository targets.
With `--repo`, these commands use normal repository and managed-path rules.
`search issues/prs/refs-to` and `activity feed` always use their repository context,
including a remote-derived repository when `--repo` is absent. Inbox provider and
query threads preserve the invocation's identity scope. Inbox cache reads and
writes are disabled under managed identity because existing snapshots have no
principal/profile binding. Repository bootstrap
continues to refuse under policy; it has no root-bootstrap identity contract. An explicit API target may be used outside a checkout. If Git
checkout metadata is present but cannot resolve, the operation refuses rather than
ignoring managed-path rules. GitLab target resolution is supported for Git-based
identity selection, including nested project paths; protected API operations remain
limited to GitHub. The local file-backed provider remains local.

Git transport resolves the actual selected remote or explicit URL and uses
`pushurl` for pushes. Multiple URLs refuse. Authoring selects `branch.pushRemote`,
then `remote.pushDefault`, then `branch.remote`, or a sole configured remote.
For a detached HEAD, authoring selects `remote.pushDefault` or a sole configured
remote; multiple remotes without an explicit repository default refuse. Probe
failures do not select a remote.
Both commit diagnostics use that same authoring selection; an explicit `--remote`
(including `--remote origin`) selects that remote instead. Unknown or ambiguous
repositories/principals refuse.

## Execution and refusal

Identity preparation applies only to the child process. It does not change
shared gh active account, persistent Git author/signing config, or stored remotes.
The explicit `git-cli transport set` command separately owns repository-local
remote URL selection; see its
[operator procedure](../../../git-cli/docs/runbooks/governed-https-transport.md).

For an HTTPS fetch/push, the guard obtains the credential from the configured
system/global Git credential helper using `git credential fill`, with interaction
and tracing disabled. Local/worktree helper entries (including URL-scoped keys,
empty resets, and values reached through local includes) refuse before helper
execution. Scope-inspection failures also refuse. Lookup runs in an owned empty
temporary directory with repository discovery fenced there; local configuration
cannot add a helper between inspection and lookup. Global/system configuration
selectors and the executable environment remain operator-owned, like the
identity policy's configuration selector. See
[Git configuration scopes](https://git-scm.com/docs/git-config#SCOPES).

The guard verifies the actor of that exact credential against the selected
profile using the same human/App checks as other protected operations. A missing
helper credential or mismatched actor refuses before transport with a fix hint;
the guard does not substitute the profile's API credential. Profile selection,
session binding, operation admission, and audit remain required.

The verified helper token is passed in the Git child's `FORGE_IDENTITY_TOKEN`
environment variable for the actual HTTPS command, without a URL rewrite or
second configured-helper lookup. SSH-shaped remotes
retain the existing profile-credential selection and child-only pinned HTTPS
routing; the stored SSH URL stays unchanged. In either case the child has an
empty credential-helper list followed by a helper constrained to the selected
HTTPS host/repository, no interactive credential fallback, and redirects disabled.

The command-scope executable overrides are exactly `core.hooksPath=/dev/null`,
`core.fsmonitor=false`, and `core.alternateRefsCommand=`. They prevent repository
hooks, fsmonitor programs, and alternate-refs commands from running with the token.

The closed network-configuration refusal list is `http.proxy`, `http.sslCAInfo`,
and `http.sslCAPath` (including their `http.<url>.*` forms), plus
`remote.<name>.proxy`. Repository/worktree
entries refuse before credential authorization with
`identity_transport_config_untrusted` and a system/global configuration fix hint.
Empty resets and entries reached through local includes also refuse, even when a
URL-specific entry does not match the selected destination. Scope-inspection
failures also refuse. This uses the same scope rules as the helper refusal;
system/global proxy and CA values remain effective. It applies to HTTPS remotes
and SSH-shaped remotes routed through pinned HTTPS.

Other repository-configured commands remain operator-owned and may inherit the
token if Git starts them. The pinned credential helper and HTTPS transport still
inherit the Git environment; their executable environment remains operator-owned.
These overrides do not change stored Git configuration or commit signing and
authoring behavior.

TLS certificate verification is enforced for the selected URL, overriding inherited
`http.sslVerify` settings and removing `GIT_SSL_NO_VERIFY` from the child.
Pre-existing URL rewrites or HTTP credential headers refuse. Credentials are never
put into argv or remote URLs. Native SSH authentication, custom ports, insecure
HTTP, local-file transport, clones, pulls, submodules, stashes, and tag authoring are unsupported
under policy and refuse.

New commits set author and committer from the profile, force OpenPGP signing with
its exact fingerprint, and verify key availability first. Conflicting repository
identity or author/committer environment overrides refuse. Amend/history-producing
operations and caller author/signing overrides refuse rather than silently
rewriting attribution. This includes attached/abbreviated signing-key options and
message-reuse options that retain another commit's author. Local `merge --ff-only` preserves existing commits and
remains available. The existing default-branch/signing/delivery gates remain
independent. An installed policy does not authorize a commit or provider operation.

`semantic-commit rebase` is the dedicated linear replay exception. Its finite
entry point requires a configured identity policy, a non-default managed linked
worktree and a verified original branch. The shared replay executor rechecks that
branch against attached HEAD or the saved merge-backend branch and original tip,
and uses its remote precedence during detached continuation. It preserves the
original authors and messages while pinning the authenticated profile's committer
and signer. Generic `prepare_git` still refuses raw rebase and other history
authoring overrides.

The replay executor accepts only the owner's fixed initial replay arguments or
continuation. Saved continuation must enable keyless signing and contain ordinary
pick commands; explicit saved keys, exec/merge programs, autostash and ref updates
refuse before execution. Authorization is renewed for each invocation. A started
conflict or signing failure retains Git's resulting recovery state; it is not
automatically aborted or retried. See the
[`semantic-commit` replay interface](../../../semantic-commit/README.md#rebase).

`git-cli open pr` uses one selected collaboration repository when it contacts gh;
a credential refusal does not try another identity or ambient repository.
Raw external `git`/`gh` invocations outside these managed runners are not covered.

Credential, actor, Git metadata, and signer probes share the caller's deadline
with execution where the runner supplies one. Standalone identity preparation
and diagnostics have a finite 30-second probe budget. Each probe bounds captured
stdout and stderr to 8 MiB, kills its process group on refusal, and reaps its leader;
probe errors contain stable codes rather than captured child output.

## Actor verification reuse

A successful actor verification is reused for the life of the process, so a
long-running command (`pr wait-checks`, the `pr merge` queue poll) probes the
actor once instead of before every managed `gh` call. The reuse is process-local
and in memory only; nothing is written to disk.

- **What is cached:** the verified actor and the time it was verified, keyed by a
  one-way SHA-256 digest of the credential, host, expected login (for an App
  profile also the app slug, app id and the repository whose installation
  coverage was checked) and the audit state root. The credential itself is never
  stored or logged.
- **Only success is cached.** An actor mismatch, missing coverage or any probe
  failure leaves no entry and is verified again on the next call. A verification
  is remembered only after its `authorized` audit record is written, so a cached
  verification was always audited; if that write fails the call is refused and
  the next call probes again.
- **Invalidation:** an entry expires after 10 minutes; a different credential,
  host, expected identity or state root is a different key and verifies again; and
  a managed `gh` call that fails with HTTP 401 or 403 drops the entry so the next
  call re-verifies. Concurrent managed calls in one process share one probe.
- **Audit:** the `authorized` record carries `actor_verification` with `cached`
  (`true` when a prior verification was reused) and `verified_at`, the time of the
  verification that was used.

Separate processes do not share verifications; each probes once.

## Diagnostics and audit

```text
forge-cli --repo example/project identity explain --operation api-read
forge-cli --repo example/project identity doctor --operation commit
```

`--operation` values: `api-read`, `api-write`, `git-read`, `git-push`, `commit`.
`explain` is strictly read-only: no credential reads, provider calls, or audit
writes. It reports enforcement, principal, matched rule, profile, and target.
`doctor` makes read-only credential/actor/key probes and records private audit;
it creates no provider object and changes no credentials or identity settings.
Both support JSON envelopes `cli.forge-cli.identity.explain.v1` and
`cli.forge-cli.identity.doctor.v1`. With no policy, they report `enforced: false`.

Private audit is `${XDG_STATE_HOME:-$HOME/.local/state}/forge-cli/identity-audit.jsonl`
(mode 0600). Records use `forge.identity.audit.v1`, with policy version/digest,
principal/session reference, repository, operation, rule/profile, observed actor,
how the actor was verified (`actor_verification`, on `authorized` records),
selected signer, and authorization/refusal/execution outcome. Captured successful
new commits include the commit SHA. Refusals before target resolution have a null
target. No credential values, raw source lines, subprocess arguments, URLs with
credentials, or provider response bodies are retained. Audit inability refuses
before execution; `identity_audit_failed_after_execution` explicitly denotes
partial success if an execution completed before a subsequent append failed.

Typed refusal codes include `identity_policy_invalid`, `identity_policy_version`,
`identity_principal_missing`, `identity_principal_unknown`,
`identity_repository_unknown`, `identity_unasserted_noninteractive`, `identity_rule_ambiguous`, `identity_path_conflict`,
`identity_operation_denied`, `identity_target_unknown`, `identity_target_ambiguous`,
`identity_gitlab_host_not_configured_add_gitlab_hosts`,
`identity_credential_missing`, `identity_actor_unavailable`,
`identity_actor_mismatch`, `identity_app_repository_missing`,
`identity_signing_key_missing`, `identity_commit_mismatch`, and transport/override
refusals. `forge-cli` maps policy refusals to DATA (65);
`identity_probe_timeout` and `identity_probe_output_limit` map to UNAVAILABLE (69).
An `identity_actor_unavailable` JSON error retains bounded
`details.retryable`, `details.next_action`, and `details.recovery` fields, plus a
`details.cause` containing the lookup operation, stage, classification, attempt,
and (when available) HTTP status, numeric `Retry-After` of at most 30 seconds,
and a validated GitHub request ID. Tokens, response bodies, stderr and command
arguments are never included. Actor probes retry HTTP 429 and 5xx failures and a
`gh` process failure that returned no HTTP response (`nonzero_exit`), at most
three attempts within the existing shared probe deadline; each reports
`retryable=true`. 401/403, actor mismatch, invalid JSON and a probe that cannot
be started are not retried.
Git runner callers retain their
existing error envelopes/exit mapping and receive stable codes in diagnostics.
TOML source diagnostics and credential-probe output are suppressed. Protected
subprocess stdout/stderr redact the selected credential before callers receive it.

## Validation and rollout

Fixture coverage includes different principals on one repository, precedence,
path conflicts, explicit defaults, operation denial, strict-schema/source canaries,
named credential lookup, missing credentials without fallback, human/App actor and
coverage checks, selected push URLs, linked worktrees, constrained helpers, and
an ephemeral-key signed commit with author/committer/signer read-back.

Phase 2 binds the launch interface to authenticated session lineage and managed
entrypoints. Phase 3 installs private metadata, provisions credentials/signers
through their existing owners, and performs disposable-repository concurrent
identity and rollout/rollback acceptance. No private policy or host installation
is included here. Removing the policy restores the existing managed behavior.

Credential backend references:
[gh named-token lookup](https://cli.github.com/manual/gh_auth_token),
[gh environment precedence](https://cli.github.com/manual/gh_help_environment),
[Git credential helpers](https://git-scm.com/docs/gitcredentials), and
[App installation authentication](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation).
