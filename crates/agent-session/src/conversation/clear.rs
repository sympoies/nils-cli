//! The receipt and input fence survive the native observer's record-lock handoff.
use super::*;
use std::path::{Path, PathBuf};

const INPUT_LOCK: &str = "conversation-clear-input.lock";

fn input_file(dir: &Path, create: bool) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(dir.join(INPUT_LOCK))
}

/// Quiescence already holds the registry lock. Clear takes the same lock
/// before claiming input, so a wake cannot pass between the check and the fence.
pub(crate) fn input_fenced(dir: &Path) -> bool {
    match input_file(dir, false) {
        Ok(file) => !lock_input(&file),
        Err(err) => err.kind() != std::io::ErrorKind::NotFound,
    }
}

fn lock_input(file: &fs::File) -> bool {
    // SAFETY: flock borrows the valid descriptor owned by file.
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

pub(crate) fn ensure_input_available(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    if input_fenced(&session_dir(context, &record.id)) {
        return Err(error(
            "conversation-clear-in-progress",
            "the conversation clear owns session input",
            record,
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub(super) struct Receipt {
    schema_version: &'static str,
    clear_id: String,
    id: String,
    session_incarnation: String,
    provider: String,
    old_provider_session_id: Option<String>,
    new_provider_session_id: Option<String>,
    prompt_sha256: Option<String>,
    // Null means a submit was attempted but could not be positively reported.
    prompt_sent: Option<bool>,
    outcome: &'static str,
    error_code: Option<String>,
    started_at: String,
    clear_submitted_at: Option<String>,
    clear_confirmed_at: Option<String>,
    prompt_submitted_at: Option<String>,
    finished_at: Option<String>,
}

pub(super) struct Attempt {
    path: PathBuf,
    receipt: Receipt,
    prompt: Option<String>,
    input_fence: Option<fs::File>,
}

impl Attempt {
    pub(super) fn start(
        context: &CliContext,
        record: &SessionRecord,
        options: &ClearOptions,
    ) -> Result<Self, CliError> {
        let prompt = options
            .initial_prompt_file
            .as_deref()
            .map(|path| read_prompt(path, record))
            .transpose()?;
        let clear_id = uuid::Uuid::new_v4().to_string();
        let dir = session_dir(context, &record.id).join("clear-receipts");
        crate::ensure_private_dir(&dir)?;
        let attempt = Self {
            path: dir.join(format!("{clear_id}.json")),
            receipt: Receipt {
                schema_version: "agent-session.clear-receipt.v1",
                clear_id,
                id: record.id.clone(),
                session_incarnation: coordination::incarnation(record)?,
                provider: record.agent.clone(),
                old_provider_session_id: record
                    .provider_resume
                    .as_ref()
                    .map(|r| r.session_id.clone()),
                new_provider_session_id: None,
                prompt_sha256: prompt
                    .as_ref()
                    .map(|text| coordination::digest_bytes(text.as_bytes())),
                prompt_sent: Some(false),
                outcome: "admitting",
                error_code: None,
                started_at: Timestamp::now().to_string(),
                clear_submitted_at: None,
                clear_confirmed_at: None,
                prompt_submitted_at: None,
                finished_at: None,
            },
            prompt,
            input_fence: None,
        };
        attempt.write()?;
        Ok(attempt)
    }

    pub(super) fn fence_input(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        crate::restart::ensure_not_fenced(context, record)?;
        let locked = coordination::lock_registry_observational(context)?;
        coordination::ensure_notification_submission_not_in_progress(
            &locked.registry,
            &record.id,
            &self.receipt.session_incarnation,
        )?;
        let file = input_file(&session_dir(context, &record.id), true).map_err(|_| {
            error(
                "conversation-operation-unavailable",
                "conversation input fence is unavailable",
                record,
            )
        })?;
        if !lock_input(&file) {
            return Err(error(
                "conversation-operation-busy",
                "another clear owns conversation input",
                record,
            ));
        }
        self.input_fence = Some(file);
        Ok(())
    }

    pub(super) fn submitted_clear(&mut self) -> Result<(), CliError> {
        self.receipt.outcome = "clear-submission-unknown";
        self.receipt.clear_submitted_at = Some(Timestamp::now().to_string());
        self.write()
    }

    pub(super) fn confirmed_clear(&mut self, session_id: &str) -> Result<(), CliError> {
        self.receipt.outcome = "clear-confirmed";
        self.receipt.new_provider_session_id = Some(session_id.into());
        self.receipt.clear_confirmed_at = Some(Timestamp::now().to_string());
        self.write()
    }

    pub(super) fn submit_prompt(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
        tmux: &Path,
        lock: crate::SessionRecordLock,
    ) -> Result<(), CliError> {
        let Some(prompt) = self.prompt.as_deref() else {
            return Ok(());
        };
        let deadline = Instant::now() + crate::send::guarded_hold_budget(record, false);
        check_composer(context, record, tmux, deadline)?;
        let mut manual = crate::ManualInputSection::new(lock);
        if record.agent == "codex" {
            codex_app_server::ensure_manual_input_capability(context, record)?;
            crate::codex_account::authorize_terminal_input_locked(context, &mut record.clone())?;
        }
        self.receipt.outcome = "prompt-submission-unknown";
        self.receipt.prompt_submitted_at = Some(Timestamp::now().to_string());
        self.receipt.prompt_sent = None;
        self.write()?;
        crate::send::input::send_conversation_input_unlocked(
            context,
            record,
            Some(prompt),
            &[SpecialKey::Enter],
            tmux,
            Some(&mut manual),
            crate::PasteMode::Bracketed,
        )
        .map_err(|_| error(
            "conversation-initial-prompt-unconfirmed",
            "initial prompt submission is unconfirmed; inspect the bound conversation before sending again",
            record,
        ))?;
        self.receipt.prompt_sent = Some(true);
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        result: Result<ResultView, CliError>,
    ) -> Result<ResultView, CliError> {
        self.receipt.finished_at = Some(Timestamp::now().to_string());
        self.receipt.outcome = match &result {
            Ok(_) => "completed",
            Err(err)
                if matches!(
                    err.code(),
                    "conversation-clear-unconfirmed" | "conversation-idle-timeout"
                ) =>
            {
                "timed-out"
            }
            Err(_) if self.receipt.outcome == "admitting" => "refused",
            Err(_) => self.receipt.outcome,
        };
        self.receipt.error_code = result.as_ref().err().map(|err| err.code().to_owned());
        let path = crate::display_path(&self.path);
        if let Err(mut err) = self.write() {
            let details = err.details_mut().get_or_insert_with(|| json!({}));
            details["receipt_path"] = json!(path);
            return Err(err);
        }
        match result {
            Ok(mut view) => {
                view.receipt_path = Some(path);
                view.receipt = Some(self.receipt);
                Ok(view)
            }
            Err(mut err) => {
                let details = err.details_mut().get_or_insert_with(|| json!({}));
                details["receipt_path"] = json!(path);
                details["receipt"] = json!(self.receipt);
                Err(err)
            }
        }
    }

    fn write(&self) -> Result<(), CliError> {
        write_private_file(
            &self.path,
            &serde_json::to_vec(&self.receipt).expect("clear receipt serializes"),
        )
    }
}

/// Enhanced Claude admission runs with the record, activity, and ingress
/// locks held. Until the shared native idle guard lands (#4183), this draft
/// retains the existing projected-idle admission and only checks the composer
/// here. The shared guard will bracket this capture with native observations.
pub(super) fn check_claude_admission(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    deadline: Instant,
) -> Result<(), CliError> {
    check_composer(context, record, tmux, deadline)
}

fn read_prompt(path: &Path, record: &SessionRecord) -> Result<String, CliError> {
    let invalid = || {
        error(
            "conversation-initial-prompt-invalid",
            "the initial prompt must be a nonempty UTF-8 text file within the prompt size limit",
            record,
        )
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| invalid())?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(invalid());
    }
    let max = crate::provider_prompt::MAX_PROVIDER_PROMPT_BYTES;
    let mut text = String::new();
    file.take(max as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|_| invalid())?;
    if text.len() > max
        || text.trim().is_empty()
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid());
    }
    Ok(text)
}
