//! Durable resume identity for the current Codex app-server runtime.
use super::*;

pub(crate) fn app_server_owns_resume_binding(record: &SessionRecord) -> bool {
    record
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.kind == RUNTIME_KIND)
}

pub(crate) fn thread_attachment_matches(record: &SessionRecord, thread_id: &str) -> bool {
    thread_attached_path(record)
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|binding| binding == projected_thread_binding(thread_id))
}

pub(super) fn persist_bound_thread_resume(
    context: &CliContext,
    expected: &SessionRecord,
    thread_id: &str,
) -> Result<(), CliError> {
    if !protocol_id_is_valid(thread_id) {
        return Err(CliError::data(
            "codex-app-server-resume-identity-invalid",
            "Codex app-server returned an invalid thread identity",
            Some(json!({ "id": expected.id })),
        ));
    }
    let resume_args = canonical_provider_resume_args(AgentKind::Codex, &expected.cwd, thread_id)
        .expect("Codex resume arguments are supported");
    let captured_at = Timestamp::now().to_string();
    mutate_session_record(context, &expected.id, |current| {
        crate::ensure_same_session_identity(expected, current)?;
        let expected_socket = socket_path(expected);
        if current.cwd != expected.cwd
            || expected_socket.is_none()
            || socket_path(current) != expected_socket
            || thread_attached_path(current) != thread_attached_path(expected)
        {
            return Err(CliError::data(
                "codex-app-server-resume-runtime-conflict",
                "Codex app-server runtime no longer matches the session",
                Some(json!({ "id": current.id })),
            ));
        }
        let has_matching_resume = if let Some(existing) = current.provider_resume.as_ref() {
            if existing.provider != AgentKind::Codex.as_str() || existing.session_id != thread_id {
                return Err(CliError::data(
                    "codex-app-server-resume-identity-conflict",
                    "Codex app-server thread conflicts with durable resume metadata",
                    Some(json!({
                        "id": current.id,
                        "provider": existing.provider,
                    })),
                ));
            }
            true
        } else {
            false
        };
        // The controller obtained this ID from the runtime's loaded list.
        // Recheck the attachment under the record lock used for the write.
        if !thread_attachment_matches(current, thread_id) {
            return Err(CliError::runtime(
                "codex-app-server-thread-binding-failed",
                "Codex app-server thread did not match the current runtime attachment",
                Some(json!({ "id": current.id })),
            ));
        }
        if has_matching_resume {
            return Ok(());
        }
        current.provider_resume = Some(ProviderResume {
            provider: AgentKind::Codex.as_str().to_string(),
            session_id: thread_id.to_string(),
            captured_at: captured_at.clone(),
            capture_method: PROVIDER_RESUME_CAPTURE_METHOD.to_string(),
            resume_args: resume_args.clone(),
            extra: BTreeMap::new(),
        });
        current.updated_at = captured_at.clone();
        Ok(())
    })
}
