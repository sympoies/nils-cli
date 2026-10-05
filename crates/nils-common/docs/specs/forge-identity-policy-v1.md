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
Repository keys are lowercase `host/owner/repository`; organization keys are
`host/owner`. Hosts have no userinfo, port, scheme, or path. The GitHub SSH hostname
alias canonicalizes to the API hostname. Configuration contains metadata and
credential **reference names**, never tokens or private keys.

```toml
version = 1

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
Cross-repository discovery (`activity`, `inbox`, `search`) and repository bootstrap
refuse when policy is installed; phase 1 has no multi-target or root-bootstrap
identity contract. An explicit API target may be used outside a checkout. If Git
checkout metadata is present but cannot resolve, the operation refuses rather than
ignoring managed-path rules. Other forge providers refuse protected network operations
under this GitHub policy; the local file-backed provider remains local.

Git transport resolves the actual selected remote or explicit URL and uses
`pushurl` for pushes. Multiple URLs refuse. Authoring selects `branch.pushRemote`,
then `remote.pushDefault`, then `branch.remote`, or a sole configured remote.
Both commit diagnostics use that same authoring selection; an explicit `--remote`
(including `--remote origin`) selects that remote instead. Unknown or ambiguous
repositories/principals refuse.

## Execution and refusal

Changes apply only to the child process. They do not change shared gh active
account, persistent Git author/signing config, or stored remotes.

Supported HTTPS and GitHub SSH remote shapes execute over pinned HTTPS with the
selected verified token. The SSH URL stays stored unchanged. The child has an
empty credential-helper list followed by a helper constrained to the selected
HTTPS host/repository, no interactive credential fallback, and redirects disabled.
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

`git-cli open pr` uses one selected collaboration repository when it contacts gh;
a credential refusal does not try another identity or ambient repository.
Raw external `git`/`gh` invocations outside these managed runners are not covered.

Credential, actor, Git metadata, and signer probes share the caller's deadline
with execution where the runner supplies one. Standalone identity preparation
and diagnostics have a finite 30-second probe budget. Each probe bounds captured
stdout and stderr to 8 MiB, kills its process group on refusal, and reaps its leader;
probe errors contain stable codes rather than captured child output.

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
selected signer, and authorization/refusal/execution outcome. Captured successful
new commits include the commit SHA. Refusals before target resolution have a null
target. No credential values, raw source lines, subprocess arguments, URLs with
credentials, or provider response bodies are retained. Audit inability refuses
before execution; `identity_audit_failed_after_execution` explicitly denotes
partial success if an execution completed before a subsequent append failed.

Typed refusal codes include `identity_policy_invalid`, `identity_policy_version`,
`identity_principal_missing`, `identity_principal_unknown`,
`identity_repository_unknown`, `identity_rule_ambiguous`, `identity_path_conflict`,
`identity_operation_denied`, `identity_target_unknown`, `identity_target_ambiguous`,
`identity_credential_missing`, `identity_actor_unavailable`,
`identity_actor_mismatch`, `identity_app_repository_missing`,
`identity_signing_key_missing`, `identity_commit_mismatch`, and transport/override
refusals. `forge-cli` maps policy refusals to DATA (65);
`identity_probe_timeout` and `identity_probe_output_limit` map to UNAVAILABLE (69).
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
