use serde::Serialize;

use crate::screen_record_adapter::{AppInfo, WindowInfo};

#[derive(Debug, Clone, Serialize)]
pub struct SuccessEnvelope<T>
where
    T: Serialize,
{
    pub schema_version: u8,
    pub ok: bool,
    pub command: &'static str,
    pub result: T,
}

impl<T> SuccessEnvelope<T>
where
    T: Serialize,
{
    pub fn new(command: &'static str, result: T) -> Self {
        Self {
            schema_version: 1,
            ok: true,
            command,
            result,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionMeta {
    pub action_id: String,
    pub elapsed_ms: u64,
    pub dry_run: bool,
    pub retries: u8,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ActionPolicyResult {
    pub dry_run: bool,
    pub retries: u8,
    pub retry_delay_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WindowRow {
    pub window_id: u32,
    pub owner_name: String,
    pub window_title: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub on_screen: bool,
    pub active: bool,
    pub owner_pid: i32,
    pub z_order: usize,
}

impl From<&WindowInfo> for WindowRow {
    fn from(window: &WindowInfo) -> Self {
        Self {
            window_id: window.id,
            owner_name: window.owner_name.clone(),
            window_title: window.title.clone(),
            x: window.bounds.x,
            y: window.bounds.y,
            width: window.bounds.width,
            height: window.bounds.height,
            on_screen: window.on_screen,
            active: window.active,
            owner_pid: window.owner_pid,
            z_order: window.z_order,
        }
    }
}

impl WindowRow {
    pub fn tsv_line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.window_id,
            normalize_tsv_field(&self.owner_name),
            normalize_tsv_field(&self.window_title),
            self.x,
            self.y,
            self.width,
            self.height,
            if self.on_screen { "true" } else { "false" }
        )
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AppRow {
    pub app_name: String,
    pub pid: i32,
    pub bundle_id: String,
}

impl From<&AppInfo> for AppRow {
    fn from(app: &AppInfo) -> Self {
        Self {
            app_name: app.name.clone(),
            pid: app.pid,
            bundle_id: app.bundle_id.clone(),
        }
    }
}

impl AppRow {
    pub fn tsv_line(&self) -> String {
        format!(
            "{}\t{}\t{}",
            normalize_tsv_field(&self.app_name),
            self.pid,
            normalize_tsv_field(&self.bundle_id)
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ListWindowsResult {
    pub windows: Vec<WindowRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListAppsResult {
    pub apps: Vec<AppRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreenshotResult {
    pub path: String,
    pub target: WindowRow,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaitResult {
    pub condition: &'static str,
    pub attempts: u32,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowActivateResult {
    pub selected_app: String,
    pub selected_window_id: Option<u32>,
    pub wait_ms: Option<u64>,
    pub policy: ActionPolicyResult,
    pub meta: ActionMeta,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputClickResult {
    pub x: i32,
    pub y: i32,
    pub button: &'static str,
    pub count: u8,
    pub policy: ActionPolicyResult,
    pub meta: ActionMeta,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputTypeResult {
    pub text_length: usize,
    pub enter: bool,
    pub delay_ms: Option<u64>,
    pub policy: ActionPolicyResult,
    pub meta: ActionMeta,
}

#[derive(Debug, Clone, Serialize)]
pub struct InputHotkeyResult {
    pub mods: Vec<String>,
    pub key: String,
    pub policy: ActionPolicyResult,
    pub meta: ActionMeta,
}

fn normalize_tsv_field(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\t' || ch == '\n' || ch == '\r' {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::normalize_tsv_field;

    #[test]
    fn normalize_tsv_field_replaces_control_whitespace() {
        assert_eq!(normalize_tsv_field("A\tB\nC\rD"), "A B C D");
    }
}
