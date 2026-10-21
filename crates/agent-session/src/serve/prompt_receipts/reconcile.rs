//! Recover only a retained native ACK, never infer an attempt from nearby input.
use super::*;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct NativeProof {
    version: u8,
    identity: String,
    pub(super) turn_id: String,
    pub(super) fingerprint: String,
}

fn identity(record: &SessionRecord) -> Option<String> {
    if !codex_app_server::runtime_is_supported(record) {
        return None;
    }
    let runtime = record.runtime.as_ref()?;
    let provider = record.provider_resume.as_ref()?;
    let binding = match crate::codex_account::binding_snapshot(record) {
        crate::codex_account::BindingSnapshot::Unbound => json!(["unbound"]),
        crate::codex_account::BindingSnapshot::Bound { account, revision } => {
            json!(["bound", account, revision])
        }
        crate::codex_account::BindingSnapshot::Blocked => return None,
    };
    Some(store::digest(&(
        &record.id,
        &record.agent,
        &record.created_at,
        &record.tmux_session,
        &runtime.kind,
        runtime.generation,
        &runtime.launch_id,
        &runtime.tmux_session,
        &provider.provider,
        &provider.session_id,
        binding,
        codex_app_server::socket_path(record),
        codex_app_server::thread_attached_path(record),
    )))
}

impl NativeProof {
    pub(super) fn new(record: &SessionRecord, turn_id: &str, fingerprint: &str) -> Option<Self> {
        Some(Self {
            version: 1,
            identity: identity(record)?,
            turn_id: turn_id.into(),
            fingerprint: fingerprint.into(),
        })
    }

    fn matches(&self, record: &SessionRecord, receipt: &Receipt) -> bool {
        self.version == 1
            && record.id == receipt.session_id
            && record
                .runtime
                .as_ref()
                .is_some_and(|r| r.launch_id == receipt.session_incarnation)
            && identity(record).as_ref() == Some(&self.identity)
    }
}

pub(super) async fn attempt(context: &CliContext, result: ResultRecord) -> ResultRecord {
    if result.receipt.outcome != Outcome::Unknown || result.native_confirmation.is_none() {
        return result;
    }
    let original = result.clone();
    let context = context.clone();
    tokio::task::spawn_blocking(move || confirm(&context, result))
        .await
        .unwrap_or(original)
}

fn confirm(context: &CliContext, original: ResultRecord) -> ResultRecord {
    let confirmed = || -> Option<ResultRecord> {
        let proof = original.native_confirmation.as_ref()?;
        // A private proof must still belong to this exact durable claim/result.
        if !store::confirmation_is_current(context, &original) {
            return None;
        }
        let deadline = Instant::now() + crate::send_submit::HOLD_BUDGET;
        let lock = crate::acquire_session_record_lock_timed(
            context,
            &original.receipt.session_id,
            deadline.saturating_duration_since(Instant::now()),
        )
        .ok()?;
        let record = load_session_record(context, &original.receipt.session_id).ok()?;
        if !proof.matches(&record, &original.receipt) {
            return None;
        }
        let _account = codex_app_server::acquire_account_mutation_gate(context, &record.id).ok()?;
        crate::codex_account::ensure_proxy_input_allowed(&record).ok()?;
        let mut client = codex_app_server::guarded::Client::connect(&record, deadline).ok()?;
        // Provider ingress may need the record lock to hydrate the accepted input.
        drop(lock);
        if !client.confirms_input(&proof.turn_id, |text| {
            store::fingerprint(text, true) == proof.fingerprint
        }) {
            return None;
        }
        let _lock = crate::acquire_session_record_lock_timed(
            context,
            &record.id,
            deadline.saturating_duration_since(Instant::now()),
        )
        .ok()?;
        let current = load_session_record(context, &record.id).ok()?;
        if !proof.matches(&current, &original.receipt) {
            return None;
        }
        // Account gate and final identity lock cover durable publication as well.
        store::publish_confirmed(context, &original).ok().flatten()
    };
    confirmed().unwrap_or(original)
}
