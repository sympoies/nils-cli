pub(crate) fn running_identity(
    session_id: &str,
    pane_id: &str,
    pane_pid: libc::pid_t,
    process_group_id: Option<libc::pid_t>,
    launch_id: Option<&str>,
) -> Probe {
    let identity = TmuxRuntimeIdentity {
        macos_boot_id: super::capture_macos_boot_id(),
        launch_id: launch_id.map(str::to_owned),
        session_id: session_id.to_owned(),
        pane_id: pane_id.to_owned(),
        pane_pid,
        pane_start_time: None,
        process_group_id,
        process_session_id: None,
        process_session_members: Vec::new(),
        control_group_members: Vec::new(),
        control_group: None,
        cgroup_mount: None,
        pid_namespace: None,
    };
    #[cfg(target_os = "macos")]
    let identity = {
        let mut identity = identity;
        if let Ok(pinned) = super::prepare_process_runtime(&identity) {
            identity.pane_start_time = Some(pinned.macos_pane.start_time);
        }
        identity
    };
    Probe::Running(Box::new(identity))
}
