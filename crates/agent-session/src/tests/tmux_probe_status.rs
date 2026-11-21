/// Model `has-session` for the pinned fixture path without racing a shell
/// invocation against the probe budget.
pub(crate) enum Status {
    Running,
    Stopped,
    #[cfg(target_os = "linux")]
    RunningMarker(PathBuf),
}

static STATUS_PROBES: LazyLock<Mutex<std::collections::HashMap<PathBuf, Status>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

pub(crate) struct StatusGuard(PathBuf);

pub(crate) fn install_status(path: &Path, status: Status) -> StatusGuard {
    assert!(
        STATUS_PROBES
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), status)
            .is_none()
    );
    StatusGuard(path.to_path_buf())
}

impl Drop for StatusGuard {
    fn drop(&mut self) {
        STATUS_PROBES.lock().unwrap().remove(&self.0);
    }
}

pub(super) fn read_status(path: &Path) -> Option<String> {
    let probes = STATUS_PROBES.lock().unwrap();
    let running = match probes.get(path)? {
        Status::Running => true,
        Status::Stopped => false,
        #[cfg(target_os = "linux")]
        Status::RunningMarker(marker) => marker.exists(),
    };
    Some(if running { "running" } else { "stopped" }.to_string())
}

/// An absent fake pane has no identity to capture. Live panes still use the
/// real identity probe and process-boundary checks.
#[cfg(target_os = "linux")]
pub(super) fn stopped_running_marker(path: &Path) -> bool {
    matches!(
        STATUS_PROBES.lock().unwrap().get(path),
        Some(Status::RunningMarker(marker)) if !marker.exists()
    )
}
