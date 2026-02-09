#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    pub id: u32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u32,
    pub owner_name: String,
    pub title: String,
    pub bounds: Rect,
    pub on_screen: bool,
    pub active: bool,
    pub owner_pid: i32,
    pub z_order: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppInfo {
    pub name: String,
    pub pid: i32,
    pub bundle_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct ShareableContent {
    pub displays: Vec<DisplayInfo>,
    pub windows: Vec<WindowInfo>,
    pub apps: Vec<AppInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PermissionState {
    Ready,
    Blocked,
    #[default]
    Unknown,
}

impl PermissionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_blocked(self) -> bool {
        matches!(self, Self::Blocked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionStatusSchema {
    pub screen_recording: PermissionState,
    pub accessibility: PermissionState,
    pub automation: PermissionState,
    pub ready: bool,
    pub hints: Vec<String>,
}

impl PermissionStatusSchema {
    pub fn from_components(
        screen_recording: PermissionState,
        accessibility: PermissionState,
        automation: PermissionState,
        hints: Vec<String>,
    ) -> Self {
        Self {
            screen_recording,
            accessibility,
            automation,
            ready: Self::compute_ready(screen_recording, accessibility, automation),
            hints: stable_unique_hints(hints),
        }
    }

    pub fn compute_ready(
        screen_recording: PermissionState,
        accessibility: PermissionState,
        automation: PermissionState,
    ) -> bool {
        let any_ready = matches!(screen_recording, PermissionState::Ready)
            || matches!(accessibility, PermissionState::Ready)
            || matches!(automation, PermissionState::Ready);

        any_ready
            && !screen_recording.is_blocked()
            && !accessibility.is_blocked()
            && !automation.is_blocked()
    }
}

impl Default for PermissionStatusSchema {
    fn default() -> Self {
        Self::from_components(
            PermissionState::Unknown,
            PermissionState::Unknown,
            PermissionState::Unknown,
            Vec::new(),
        )
    }
}

fn stable_unique_hints(hints: Vec<String>) -> Vec<String> {
    let mut unique = Vec::with_capacity(hints.len());
    for hint in hints {
        if !unique.iter().any(|existing| existing == &hint) {
            unique.push(hint);
        }
    }
    unique
}

pub const RECORDING_DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;
pub const RECORDING_DIAGNOSTICS_CONTRACT_VERSION: &str = "1.0";
pub const RECORDING_DIAGNOSTICS_ARTIFACT_DIR_SUFFIX: &str = "diagnostics";
pub const CONTACT_SHEET_ARTIFACT_SUFFIX: &str = "contact-sheet.svg";
pub const MOTION_INTERVALS_ARTIFACT_SUFFIX: &str = "motion-intervals.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingDiagnosticsArtifacts {
    pub contact_sheet_path: PathBuf,
    pub motion_intervals_path: PathBuf,
    pub interval_count: usize,
}
use std::path::PathBuf;
