//! The Codex 0.171 app-server names the rollout before its first turn writes it.
use super::*;
use std::path::{Component, PathBuf};

#[cfg(test)]
#[path = "first_tests.rs"]
mod tests;

pub(crate) struct LiveRollout {
    config_dir: PathBuf,
    relative: PathBuf,
    thread_id: String,
}

impl LiveRollout {
    pub(crate) fn observe(record: &SessionRecord, provider_path: &Path) -> Option<Self> {
        let thread = record.provider_resume.as_ref()?;
        let root = crate::session_provider_config_dir(record)
            .map(|dir| dir.join("sessions"))
            .or_else(super::super::codex_sessions_root)?;
        let config_dir = fs::canonicalize(root.parent()?).ok()?;
        let relative = provider_path
            .strip_prefix(&root)
            .or_else(|_| provider_path.strip_prefix(config_dir.join("sessions")))
            .ok()?;
        let name = relative.file_name()?.to_str()?;
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
            || !name.starts_with("rollout-")
            || !name.ends_with(&format!("-{}.jsonl", thread.session_id))
        {
            return None;
        }
        Some(Self {
            config_dir,
            relative: relative.to_owned(),
            thread_id: thread.session_id.clone(),
        })
    }

    pub(crate) fn pin(&self) -> Option<Baseline> {
        let path = self.path(true)?;
        let source = ProviderPromptSource::for_history("codex", &self.thread_id, path)?;
        Baseline::pin(source.validated_path()?.to_owned())
    }

    pub(crate) fn is_absent(&self) -> bool {
        self.path(false).is_some()
    }

    fn path(&self, present: bool) -> Option<PathBuf> {
        if fs::canonicalize(&self.config_dir).ok()? != self.config_dir {
            return None;
        }
        let mut path = self.config_dir.clone();
        let mut components = Path::new("sessions").join(&self.relative);
        let leaf = components.file_name()?.to_owned();
        components.pop();
        for part in &components {
            path.push(part);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() => {}
                Err(error) if !present && error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return None,
            }
        }
        path.push(leaf);
        match fs::symlink_metadata(&path) {
            Err(error) if !present && error.kind() == std::io::ErrorKind::NotFound => Some(path),
            Ok(metadata) if present && metadata.is_file() => Some(path),
            _ => None,
        }
    }

    pub(crate) fn confirm(
        &self,
        id: &str,
        text: &str,
        turn: &str,
        deadline: Instant,
    ) -> Option<Proof> {
        loop {
            if Instant::now() >= deadline {
                return None;
            }
            if let Some(mut baseline) = self.pin() {
                // Scan from the provider-created file's beginning, retaining its
                // device and inode for the same checks as an existing baseline.
                baseline.stamp.eof = 0;
                baseline.stamp.continuity_sha256 = sha256(&[]);
                return super::confirm_from_origin(
                    &mut baseline,
                    id,
                    text,
                    turn,
                    &self.thread_id,
                    deadline,
                    true,
                );
            }
            if self.path(false).is_none() && self.path(true).is_none() {
                return None;
            }
            std::thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

/// Codex 0.171 persists initial instructions and environment context as a
/// user-role response. Require explicit provider types on the acknowledged turn.
pub(super) fn startup_context(value: &Value, turn: &str) -> bool {
    let data = &value["payload"];
    let metadata = &data["internal_chat_message_metadata_passthrough"];
    if metadata["turn_id"].as_str() != Some(turn) {
        return false;
    }
    let (Some(parts), Some(kinds)) = (
        data["content"].as_array(),
        metadata["content_item_kinds"].as_array(),
    ) else {
        return false;
    };
    !kinds.is_empty()
        && kinds.len() == parts.len()
        && kinds.iter().all(|kind| {
            matches!(
                kind.as_str(),
                Some("agents_md.instructions" | "environments.environment_context")
            )
        })
        && parts
            .iter()
            .all(|part| part["type"] == "input_text" && part["text"].is_string())
}
