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

Expect **17 passed**: 13 Claude recheck/schedule cases and four Codex
queued-repair cases. An empty selection fails. Any failure leaves this
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
| Claude repaired | `auth_recheck_settles_the_quarantine_once_credentials_are_republished` records `credentials_refreshed`, preserves the runtime and admits a synthetic `send` to the fake terminal. |
| Claude unrepaired | `auth_recheck_refuses_while_credentials_are_unrepaired` covers unchanged publication and missing synthetic credentials; receipts stay unchanged and admission returns `auth-quarantined`. |
| Claude automatic recovery | `daemon_settles_auth_quarantine_when_same_account_credentials_are_usable` exercises the daemon tick; the remaining schedule cases cover exhaustion, carried quarantine, publication fences, account mismatch and preserving queued intent. |
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
