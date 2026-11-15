# Authentication quarantine fixture acceptance

Use this headless recipe for [#3005](https://github.com/sympoies/nils-cli/issues/3005)
when live credentials cannot be revoked or altered. It exercises the existing
recovery implementations with disposable state, synthetic credentials and fake
provider/control responses. It requires no account login, running session,
daemon, desktop or provider connection.

## Run the fixtures

From the repository root on Linux, use the repository Rust toolchain,
`cargo-nextest`, Python 3 and a working systemd user manager with cgroup v2.
The Claude fake-runtime fixture also requires `setsid` on `PATH`.
See the [workspace maintenance reference](../../../../docs/runbooks/workspace-maintenance-reference.md#gate-resource-budgets)
for resource admission prerequisites. The gate queues behind other checks,
applies its memory budget and clamps build concurrency to its runner limit.

```bash
source scripts/dev-env.sh
CARGO_BUILD_JOBS=4 python3 scripts/ci/gate-resources.py -- \
  cargo build --locked -p nils-agent-session --bin agent-session
CARGO_BUILD_JOBS=4 python3 scripts/ci/gate-resources.py -- \
  cargo nextest run --locked -p nils-agent-session --lib \
  --no-tests fail --test-threads 1 \
  -E 'test(/^restart::tests::auth_recovery_tests::recheck::/) | test(/^serve::account_drain::tests::queued_same_account_recheck_/)'
```

Build and test with the same `CARGO_TARGET_DIR` (leave it unset for the Cargo
default). The unit-test launcher requires the matching `debug/agent-session`
binary for its disposable coordination broker, even with an installed CLI on
`PATH`.

Expect the selected Claude recheck/schedule cases and four Codex queued-repair
cases to pass; record the current selected count. An empty selection fails. Any failure leaves this
acceptance incomplete; retain the failing test name and typed result rather
than retrying against a real account.

The tests allocate temporary session state and provider configuration. The
Claude broker returns a temporary config directory containing a synthetic
`.credentials.json` and publication metadata; the fake runtime records input
locally. The Codex broker returns fixed fixture-only access credentials, and an
in-process control channel simulates successful or refused binding. They do
not inspect the operator's credential files or contact either provider. Test
teardown owns the temporary files and fake pane processes; no manual credential
setup or cleanup is needed.

## Interpret the result

| Case | Expected evidence |
| --- | --- |
| Claude repaired | `auth_recheck_requires_runtime_adoption_after_credentials_are_republished` keeps the failed resident quarantined after publication, then exercises guarded same-account restart and fresh synthetic input. |
| Claude unrepaired | `auth_recheck_refuses_while_credentials_are_unrepaired` covers unchanged publication and missing synthetic credentials; receipts stay unchanged and admission returns `auth-quarantined`. |
| Claude automatic recovery | `daemon_recheck_keeps_failed_resident_quarantined_after_publication` exercises the daemon tick; the remaining schedule cases cover exhaustion, carried quarantine, publication fences, account mismatch and preserving queued intent. |
| Codex repaired | `queued_same_account_recheck_settles_after_confirmed_bind` starts with `apply_failed` and `blocked_no_credential`, queues the same account, runs daemon repair without a prompt, and admits input only after a confirmed bind settles quarantine. |
| Codex unrepaired | `queued_same_account_recheck_refusal_keeps_quarantine` retains failed binding and quarantine after broker refusal. |
| Codex concurrent/storage failure | The other two queued-repair cases preserve a newer authentication failure and prove retry after incident-store failure. |

On current main, [#3475](https://github.com/sympoies/nils-cli/pull/3475)
supplies Codex settlement after confirmed queued repair, and
[#3515](https://github.com/sympoies/nils-cli/pull/3515) supplies bounded Claude
automatic rechecks. A credential pull alone is not proof of a successful Codex
bind. The operator's explicit queued repair owns account mutation; the Claude
automatic recheck only checks credentials and never changes account selection
or queued intent.

## Tester handoff

Report the checkout commit, the command above, the selected/passed/failed test
counts and whether the fixtures ran without live credential operations. These
results establish synthetic recovery and admission behavior. They do not prove
native provider authentication or delivery of a real model turn: Codex binding
is simulated, and Claude `send` reaches a fake terminal.

Keep any live post-release verification separate and assigned to the scheduled
tester with an approved disposable account. The tester must verify actual
native binding and subsequent input after credential repair. This recipe does
not require creating a real quarantine or manipulating real credentials.
Person/watchdog notification transport is also separate; see the
[authentication alert delivery contract](../specs/auth-alert-delivery-v1.md).

## Publication and recovery acceptance (#2954)

The `publication_acceptance` module adds an offline publisher-to-recovery
harness for [#2954](https://github.com/sympoies/nils-cli/issues/2954). Its Python
broker is under `tests/fixtures/auth-publication/` in this crate. It accepts only
the synthetic account `alpha` and a caller-supplied disposable root. It stages
synthetic credential/configuration files, then reads back their contents and
permissions before emitting completed publication/store receipts. It never
reads a real account, contacts a provider or edits incident/registry authority.
Its fixed timestamps and identities are fixture data, not live evidence.

Run from the repository root with Rust **1.100.0**, Python 3, `setsid` and a
systemd user manager. Use one build directory for all commands:

```bash
source scripts/dev-env.sh
export CARGO_BUILD_JOBS=4
systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0 \
  cargo +1.100.0 build --locked -p nils-agent-session --bin agent-session
systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0 \
  cargo +1.100.0 test --locked -p nils-agent-session --lib \
  auth_recovery_tests -- --nocapture
systemd-run --user --scope -p MemoryMax=6G -p MemorySwapMax=0 \
  cargo +1.100.0 test --locked --workspace --lib \
  synthetic_publication_ -- --nocapture
bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast
```

The workspace-selected run must select both new tests, including their JSON
comparisons. The first focused run also covers existing authentication guards,
draft/readiness protection and unknown-effect non-replay. The local-fast gate
owns its resource admission and memory budget; do not nest it in the 6 GiB
scope. If sccache reports a socket path that is too long, repeat only the failed
command with `TMPDIR=/tmp`. Temporary stores and fake pane process groups are
owned by the fixture teardown, including on assertion failure.

| Acceptance row | Synthetic evidence | Live status |
| --- | --- | --- |
| Incident and quarantine | Normal main-thread structured hook ingress, typed `auth-quarantined`, no paste/submission | UNRUN |
| Publication prerequisite | Advertised `credential_state_v1`; runtime-bound launch checkpoint; matching identity/epoch; newer generation and different snapshot; incomplete stores cannot restart; completed verified stores can | UNRUN |
| Recovery and identity | Existing stop/resume/readiness controller; stable managed session/conversation, advanced runtime incarnation/generation; fresh guarded submission | UNRUN |
| Initial refusal prevention | Publication completed before launch; synthetic launch store and checkpoint both use generation 2; fresh input admitted without an incident | UNRUN |
| Wake outcomes/history | Existing `inspection_retains_wake_deferral_after_a_later_generation_is_submitted` checks eight bounded reason/timestamp entries after a newer submitted notification; submission is not consumption | UNRUN |
| Draft, auth and unknown effects | Existing recovery guards plus the harness's retained unknown operations, no replay and one fresh guarded send | UNRUN |
| Registry writer fence/readers | Existing `notification_history_registry_fences_older_writers_and_preserves_its_version` preserves the v5 history fence and matching reader admission; later registry features may require a stronger fence | UNRUN |
| Cleanup | Temporary-root and fake-pane ownership; existing restart fixture teardown tests | UNRUN |

The initial-prevention case models a publication-before-launch barrier. It does
not establish resident credential reread acknowledgement, provider overlap,
a successful provider request, or zero refusals during refresh. These remain
separate proofs under the [recovery contract](../specs/claude-auth-recovery-v1.md#ordinary-replacement-and-evidence-limits).
No new product behavior is implied by a passing harness.

### Live fixture owner prerequisites

The resident tester must receive all of the following before any live row runs:

- An exact supported provider binary version, profile and isolated setup command.
  Claude Code **2.1.317** is the recovery contract's audited adapter version,
  not a verified disposable fixture. The earlier placeholder/401 attempt on
  **2.1.296** did not emit the required structured event. Neither is a claim
  that a working live fixture exists.
- A fixture-owner executable, referred to below as `FIXTURE`, with a private
  manifest and bounded commands for setup, launch, failure, publication,
  observations and cleanup. The owner must map these operations to supported
  provider/runtime interfaces; they are a handoff interface, not shipped CLI
  commands. Missing operations leave the corresponding row UNRUN.
- Fixture-owned broker argv, disposable account nickname, host/authority/epoch
  identity domain and isolated provider config/state/tmux ownership. No real
  account, copied authentication file or production broker store is permitted.
- A supported way to produce a **main-thread structured** authentication failure
  through the installed provider's normal hook path. A loopback 401 alone is
  insufficient. The owner supplies the trigger command and compatible profile;
  the tester must not write an incident or registry record to simulate it.
- A supported publisher that advertises `credential_state_v1` and exposes
  `credential-state --provider claude --account <fixture-account> --format json`.
  It must complete the same-identity/epoch higher generation with a different
  snapshot, matching target/authority/last-published versions, policy and
  publication timestamps, no failure, and every required store verified.
- Content-free observation commands for admission, launch checkpoint, incident
  and recovery receipts, runtime/conversation identity, notification reasons
  and timestamps, submission versus consumption, and actual fresh provider
  request outcome. Raw credential, pane, conversation and provider payloads
  must not enter retained evidence.
- Tester-owned disposable attachment/composer controls, unique message/request
  keys, fixture-only unknown-effect probes and process/state cleanup. Initial
  refusal prevention additionally needs an actual supported overlap/request
  barrier or version-bound resident-adoption proof; publication alone cannot
  demonstrate it.

### Bounded live recipe for the resident tester

Run only after release and resource scheduling. `FIXTURE` must be an absolute
executable supplied and reviewed by the fixture owner. Its manifest binds the
exact released CLI/provider versions, broker argv, disposable root and cleanup
ownership. The commands below define the required owner handoff interface;
until that executable/manifest exists, this recipe is **UNRUN**, not runnable
with the repository's synthetic broker substituted for a live publisher.

```bash
# All paths are supplied by the fixture owner; evidence is content-free.
: "${FIXTURE:?fixture owner executable required}"
: "${FIXTURE_MANIFEST:?reviewed private manifest required}"
: "${EVIDENCE_DIR:?allocated private evidence directory required}"
"$FIXTURE" setup --manifest "$FIXTURE_MANIFEST" --timeout-seconds 30
"$FIXTURE" verify-publication --generation baseline --timeout-seconds 2
"$FIXTURE" launch --timeout-seconds 30
"$FIXTURE" observe --row launch-checkpoint --timeout-seconds 2
"$FIXTURE" fail-auth --timeout-seconds 30
"$FIXTURE" observe --row quarantine --timeout-seconds 15
"$FIXTURE" stage-publication --generation newer --timeout-seconds 15
"$FIXTURE" observe --row incomplete-publication-refused --timeout-seconds 15
"$FIXTURE" publish --generation newer --timeout-seconds 15
"$FIXTURE" verify-publication --generation newer --timeout-seconds 2
"$FIXTURE" observe --row recovery-and-identity --timeout-seconds 120
"$FIXTURE" fresh-guarded-request --timeout-seconds 30
"$FIXTURE" observe --row no-replay --timeout-seconds 2
"$FIXTURE" export-redacted --directory "$EVIDENCE_DIR" --timeout-seconds 5
"$FIXTURE" cleanup --timeout-seconds 30
"$FIXTURE" verify-clean --timeout-seconds 5
```

Every operation must stop at its deadline and return a typed outcome. On any
failure, stop that row, export only redacted metadata, then run cleanup and
verify-clean before another run. Each run has an overall **10-minute deadline**;
the fixture owner must install cleanup on exit/signals and supervise its owned
processes. Verify no process, pane, broker, store or disposable directory
survives. Cleanup refusal or unknown ownership blocks another run and goes to
the fixture owner; never broaden deletion targets.

Before publication, require an observed `auth_failed` incident on the exact
current runtime and an actual guarded refusal with `pasted: false` and
`submitted: false`. Preserve that prerequisite proof before evaluating wakes.
After publication, require the full completion proof, an advanced runtime
incarnation/generation, unchanged session/account/conversation, readiness and a
separate successful fresh provider request. Record monotonic event times for
failure, publication, adoption, submission and response; readiness/submission
must not be reported as provider success.

For wake acceptance, repeat with **fresh disposable state** for attached,
detached+empty, detached+nonempty and detached+unobservable composers. The fixture
owner must supply supported controls for these states. Use normal authenticated
mailbox delivery: attached/nonempty/unobservable cases preserve drafts and
record their fixed deferral reasons; detached+empty may submit only after all
normal guards admit it. Then deliver a different message/generation and inspect
the prior bounded reason/timestamp history. Authenticate the intended runtime
for broker status and use its normal mailbox read/ack path to prove consumption
separately from `prompt_submitted`. Check the v5-or-stronger writer fence and
compatible readers **without starting an old writer** against the registry.

For initial-refusal prevention, start another disposable run: complete the
newer publication and the owner's supported barrier/adoption operation before
the first fresh request. Require zero structured authentication refusals and a
successful provider response. If the owner supplies only restart/readiness,
mark prevention UNRUN even when recovery passes. Record unknown-effect probes
as historical evidence and prove no automatic resubmission or replay. Do not
use raw terminal input, `--force`, guard bypasses or production credentials to
fill a missing row.
