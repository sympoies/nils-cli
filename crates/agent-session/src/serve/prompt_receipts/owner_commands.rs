//! Owner composer commands are terminal input, never app-server model text.
use super::*;

/// Sealed origin proof created only by the authenticated conversation handler.
/// Generic send, notifications and peer input cannot opt into login recovery.
pub(crate) struct Command {
    text: String,
}

/// A command is a leading `/name`. A first token with another slash is an
/// absolute path (for example a session upload), which stays ordinary input;
/// this also makes a namespaced `/ns/cmd` name plain input.
pub(super) fn parse(text: &str) -> Option<Command> {
    let text = text.trim_start();
    let first = text.split_whitespace().next()?;
    (first.starts_with('/') && !first[1..].contains('/')).then(|| Command {
        text: text.to_owned(),
    })
}

impl Command {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn is_login(&self, text: Option<&str>) -> bool {
        text == Some(self.text.as_str()) && self.text.trim() == "/login"
    }

    pub(crate) fn check(&self, record: &SessionRecord) -> Result<(), CliError> {
        let value = self.text.trim();
        let name = value.split_whitespace().next().unwrap_or_default();
        if record.agent == "codex" && name == "/login" {
            return Err(CliError::runtime(
                "codex-account-login-managed",
                "Codex accounts are broker-managed; use the Console account switch instead of /login",
                Some(json!({"id":record.id, "stage":"admission",
                    "pasted":false, "submitted":false})),
            ));
        }
        // Mirrored contract: Salon packages/ui/src/terminalContextCommandGuard.ts.
        // Changes to either provider list must also update that consumer guard.
        let commands: &[&str] = match record.agent.as_str() {
            "claude" => &[
                "/clear",
                "/new",
                "/reset",
                "/resume",
                "/branch",
                "/teleport",
                "/tp",
            ],
            "codex" => &["/new", "/clear", "/resume", "/fork"],
            _ => &[],
        };
        // A bare prefix can select a context command when Enter completes it.
        let replaces_context = commands
            .iter()
            .any(|command| *command == name || (value == name && command.starts_with(name)));
        if replaces_context {
            return Err(CliError::runtime(
                "send-context-command-unsupported",
                "this command replaces the provider conversation; use the managed session lifecycle instead",
                Some(json!({"id":record.id, "stage":"admission",
                    "pasted":false, "submitted":false})),
            ));
        }
        if record.provider_resume.as_ref().is_none_or(|resume| {
            resume.provider != record.agent || resume.session_id.trim().is_empty()
        }) {
            return Err(CliError::runtime(
                "send-identity-unobservable",
                "provider conversation identity is unavailable",
                Some(json!({"id":record.id, "stage":"admission",
                    "pasted":false, "submitted":false})),
            ));
        }
        if record.agent == "codex" {
            crate::codex_account::ensure_proxy_input_allowed(record).map_err(|error| {
                let mut details = error.details().cloned().unwrap_or_else(|| json!({}));
                details["stage"] = json!("admission");
                details["pasted"] = json!(false);
                details["submitted"] = json!(false);
                CliError::runtime(error.code(), error.message(), Some(details))
            })?;
        }
        Ok(())
    }
}
