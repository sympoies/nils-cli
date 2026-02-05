#[cfg(target_os = "linux")]
mod linux_portal_unit {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use screen_record::linux::portal;

    #[test]
    fn portal_missing_error_text_is_actionable_and_deterministic() {
        let lock = GlobalStateLock::new();

        let _force_missing = EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_MISSING", "1");
        let _force_available =
            EnvGuard::remove(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_AVAILABLE");

        let err = portal::ensure_portal_available().expect_err("expected missing portal error");
        let message = err.to_string();
        assert!(message.contains("xdg-desktop-portal"));
        assert!(message.contains("Wayland-only"));
        assert!(message.contains("org.freedesktop.portal.Desktop"));
    }

    #[test]
    fn portal_acquire_test_mode_bypasses_dbus_and_returns_fixed_node_id() {
        let lock = GlobalStateLock::new();

        let _test_mode = EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_TEST_MODE", "1");
        let _force_missing = EnvGuard::remove(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_MISSING");
        let _force_available =
            EnvGuard::remove(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_AVAILABLE");

        let _dbus = EnvGuard::set(
            &lock,
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/does-not-exist",
        );

        let node_id = portal::acquire_pipewire_node_id().expect("node id");
        assert_eq!(node_id, portal::TEST_PIPEWIRE_NODE_ID);
    }
}
