//! Typed native control commands.
use super::*;

pub(crate) enum ControlCommand {
    Usage(oneshot::Sender<Result<UsageSnapshot, String>>),
    Prompt {
        message: String,
        client_message_id: Option<String>,
        write: Option<Box<mailbox_notice::Write>>,
        response: oneshot::Sender<Result<String, prompt_submit::PromptSubmissionFailure>>,
    },
    Steer {
        message: String,
        expected_turn_id: String,
        client_message_id: Option<String>,
        write: Option<Box<mailbox_notice::Write>>,
        response: oneshot::Sender<Result<String, String>>,
    },
    NoticeContinue {
        message: String,
        client_message_id: String,
        expected: Box<SessionRecord>,
        tmux_bin: PathBuf,
        recovery: crate::auto_resume::notice::Recovery,
        deadline: Instant,
        response: oneshot::Sender<Result<String, String>>,
    },
    BindAccount {
        account: String,
        revision: u64,
        response: oneshot::Sender<Result<crate::codex_account::CodexAccountView, String>>,
    },
    /// Drain and apply a queued next-account intent if the turn is idle. Used by
    /// the periodic idle-boundary drive; a no-op when nothing is drainable.
    ApplyNext {
        response: oneshot::Sender<Result<(), String>>,
    },
}

pub(super) fn prompt_request(id: u64, thread: &str, message: &str, client: Option<&str>) -> Value {
    let mut request = continuation_request(id, thread, message);
    if let Some(client) = client {
        request["params"]["clientUserMessageId"] = json!(client);
    }
    request
}
