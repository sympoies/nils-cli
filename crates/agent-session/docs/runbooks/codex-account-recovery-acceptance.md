# Codex account recovery acceptance

Use a disposable Codex session and configured test account nicknames. Run the
operator commands from a different session. Keep automatic continuation disabled
and reserve the test session so no other operator or automation sends input.
Native runtime readiness, activity phase, and account binding are separate facts.

## Startup recovery

For a session with a failed startup binding, an attached native idle thread, an
empty composer, and activity phase `starting`:

```bash
agent-session account switch "$TEST_SESSION_ID" --account "$TEST_TARGET_ACCOUNT" --format json
agent-session restart "$TEST_SESSION_ID" --format json
agent-session account show "$TEST_SESSION_ID" --format json
```

Expect a ready restart receipt, a new incarnation, and the target account bound
to that incarnation with no queued intent. Ordinary input remains blocked until
binding completes. Busy native threads, pending attention, unknown identity, and
concurrent notification submission still refuse normal restart.

## Idle fixture without a model turn

The supported `activity event --stdin` interface can seed lifecycle metadata for
an **isolated disposable acceptance session**. This simulates a completed turn;
it does not submit a model request or establish that a real quota failure has
recovered. Mark both events as `console_observation`, so they cannot report a
successful provider authentication or arm provider-hook continuation. Never use
synthetic events to clear activity on a working session.
First confirm the test runtime has an attached native idle thread, no pending
attention, and an empty composer. A synthetic snapshot cannot make a busy native
thread idle or authorize input through a failed account binding or quarantine.

Set `TEST_STATE_DIR`, `TEST_SESSION_ID`, and `TEST_TARGET_ACCOUNT` from the test
configuration. Start with activity phase `starting` and auto-resume disabled.
The following Bash recipe needs `jq` and uses the public session incarnation
from `list`; it never edits session files:

```bash
set -euo pipefail
: "${TEST_STATE_DIR:?}" "${TEST_SESSION_ID:?}" "${TEST_TARGET_ACCOUNT:?}"
as_test() { agent-session --state-dir "$TEST_STATE_DIR" "$@"; }
runtime=$(as_test list --format json | jq -er --arg id "$TEST_SESSION_ID" \
  '.data[] | select(.session_id == $id and .provider == "codex" and .runtime_status == "running") | .session_incarnation')
thread=$(as_test glance "$TEST_SESSION_ID" --tail 1 --format json | \
  jq -er '.data.provider_resume | select(.provider == "codex") | .session_id')
as_test activity status "$TEST_SESSION_ID" --format json | \
  jq -e '.data.turn_state.phase == "starting" and .data.turn_state.current_turn == null'
nonce="acceptance-idle-$(date +%s)-$$"
for kind in turn_started turn_completed; do
  jq -n --arg runtime "$runtime" --arg event "$nonce-$kind" \
    --arg thread "$thread" --arg turn "$nonce" --arg kind "$kind" \
    '{schema_version:"agent-session.turn-event.v1",event_id:$event,
      runtime_id:$runtime,provider:"codex",provider_session_id:$thread,provider_turn_id:$turn,
      kind:$kind,confidence:"observed",source_kind:"console_observation"}' | \
    as_test activity event "$TEST_SESSION_ID" --stdin --format json
done
as_test activity status "$TEST_SESSION_ID" --format json | \
  jq -e '.data.turn_state.phase == "waiting" and .data.turn_state.current_turn == null'
as_test account switch "$TEST_SESSION_ID" --account "$TEST_TARGET_ACCOUNT" --format json
as_test restart "$TEST_SESSION_ID" --format json
as_test account show "$TEST_SESSION_ID" --format json
```

The public activity name for idle is `waiting`. Incarnation changes reject stale
fixture events; rerun setup against the replacement runtime rather than reusing
an old runtime ID. Stop on any refusal and inspect it before proceeding. A
daemon may apply the queued switch at the idle boundary before restart; the
final account must still be the requested target bound to the new incarnation.
Record synthetic-idle acceptance separately from live exhausted-account
acceptance, which still requires a genuine quota condition and configured
credentials. Do not publish credential material or environment-specific values.

For headless regression coverage of startup admission without model RPCs:

```bash
cargo test -p nils-agent-session --lib restart::codex_account_tests
```
