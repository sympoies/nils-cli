# Authentication alert delivery v1

An `owner_unavailable` authentication notification creates an independently
consumable delivery record as well as the observation-spool event. This record
requires neither the affected runtime nor the serve daemon to read or
acknowledge. The configured external consumer delivers to a person and,
optionally, an independent watchdog. nils-cli never contacts those transports,
selects an account, rechecks credentials or submits input through this interface.

## Consumer contract

1. Poll `agent-session auth-alert list --format json` on the consumer's configured
   cadence. `--limit` defaults to 100 and accepts 1 through 200. Pending alerts
   come before delivered audit records; `truncated` signals a bounded result.
2. Use `alert_id` as the transport deduplication key. Join the session/incident
   selectors for diagnostics. `phase` is `incident` or `recovery`; result alerts
   have an identity bound to the incident and the specific recovery result.
3. Deliver only the alert's pending `required_targets`, using the consumer's
   configured destination for each target. Failed or unknown delivery stays
   pending and is retried by the consumer. Recording or queueing is not delivery.
4. After transport confirms delivery, retain its proof privately and acknowledge
   the target with a digest of that proof:

   ```sh
   agent-session auth-alert ack <SESSION> <ALERT_ID> --target person --receipt sha256:<64-lowercase-hex-digits> --format json
   ```

5. A repeated acknowledgement with the same digest is idempotent, including its
   timestamp. A different digest for the same target returns
   `auth-alert-receipt-conflict`. An unconfigured target returns
   `auth-alert-target-unconfigured`. Neither changes the record.

The consumer must confirm transport success before acknowledging. Acknowledgement
is a local operator assertion, not proof that a person read the alert. Run it
under the OS account authorized to access the session state; no runtime-issued
capability or live daemon is required. Transport destinations, proof payloads,
credentials and provider messages are never stored by nils-cli.

## Required targets and state

The daemon reads `AGENT_SESSION_AUTH_ALERT_TARGETS` when it creates an alert.
Unset means `person`; `person,watchdog` additionally requires watchdog delivery.
Other values are refused. The alert retains its exact `required_targets` and
per-target `delivery` map, so changing daemon configuration does not silently
change an existing delivery obligation. An unconfigured watchdog never keeps a
person-only alert pending.

Each target starts with `state: pending`, `delivered_at: null`, and
`receipt: null`. A confirmed acknowledgement sets its state to `delivered`,
records the local acknowledgement timestamp and the SHA-256 receipt digest.
The alert's combined state and `delivered_at` change only when every recorded
required target is delivered. Person and watchdog acknowledgements are independent.
The original mailbox notification's `delivered_at` stays null when no mailbox
received it; external delivery is shown by `auth-alert list`.

Each record also includes `session_id`, `incident_id`, original
`runtime_incarnation` and `runtime_generation`, provider, phase, fixed result,
`recorded_at`, and nullable `recovery_action`. Incident alerts have result
`authentication`. Recovery results are `healthy`, `credentials_refreshed`,
`refresh_failed`, `runtime_replaced`, `account_switched`, `blocked_no_credential`,
or `recheck_exhausted`. For `healthy`, `credentials_refreshed`, `account_switched`
and `runtime_replaced`, the action is null.

A `recheck_exhausted` result whose last scheduled attempt returned
`provider_verification_required` or `runtime_adoption_required` carries
`restart_same_account`: preserve any draft and use the existing guarded restart
with the same account and conversation (see the
[unchanged-publication refusal](claude-auth-recovery-v1.md#unchanged-publication-without-independent-provider-proof)
and [ordinary replacement limits](claude-auth-recovery-v1.md#ordinary-replacement-and-evidence-limits)).
Other alerts carry `same_account_credential_recheck` as operator guidance.
Neither action authorizes an automatic restart or recheck in the consumer.
A final-result alert remains separate from the original incident acknowledgement.

## Persistence and retry

Private per-session `auth-alerts.json` uses schema
`agent-session.auth-alerts.v1`. The session-record lock serializes enqueue and
acknowledgement, and writes are atomic with owner-only file mode. The bounded
store retains at most 64 alerts and 128 KiB; only delivered records can be
reclaimed for a new alert. A full pending store fails explicitly and retries on
the next daemon tick. Repeated ticks retain acknowledgements and enqueue once.

The daemon reconciles every undelivered owner-unavailable notification, including
one whose spool event is already recorded. Existing recorded incidents acquire
a delivery record on the next tick. `escalation.state: delivery_unavailable`
means the deliverable record could not be retained; the next tick retries without
repeating an already recorded spool event. The spool and delivery record have
independent availability: a failed spool write does not prevent a durable alert
from being delivered by the consumer. List and ack report fixed storage
errors and never claim delivery when persistence fails.

Release acceptance requires the integration owner to wire the consumer, confirm
an owner-unavailable incident reaches the configured person, and read back its
`delivered` acknowledgement. When watchdog is configured, confirm its receipt
independently. This repository supplies the record and acknowledgement contract;
the external consumer owns transport, retries and hands-on delivery acceptance.

## JSON envelopes and errors

Both commands use the standard CLI envelope with `ok`, `schema_version`, and
`data` on success or `error` on failure. List uses
`cli.agent-session.auth-alert.list.v1`; its `data` contains `alerts` (the records
above) and Boolean `truncated`. Ack uses `cli.agent-session.auth-alert.ack.v1`;
its `data` is the complete updated alert, including on an idempotent re-ack.
Context-resolution and argument-parse errors retain the same leaf schema.

Fixed operation errors are `auth-alert-receipt-invalid`,
`auth-alert-not-found`, `auth-alert-target-unconfigured`,
`auth-alert-receipt-conflict`, `auth-alert-store-unavailable`, and
`auth-alert-encode-failed`. Invalid daemon target configuration reports
`auth-alert-targets-invalid` internally and retains `delivery_unavailable` for
retry. Existing session-ID, session-record, context and lock errors retain their
standard codes. Error messages do not include proof values or private paths.
