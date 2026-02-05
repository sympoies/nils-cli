#[cfg(target_os = "linux")]
mod linux_portal_unit {
    use std::collections::HashMap;

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

    #[test]
    fn portal_force_available_truthy_values_override_force_missing() {
        let lock = GlobalStateLock::new();

        for value in ["1", "true", " yes ", "On"] {
            let _force_available =
                EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_AVAILABLE", value);
            let _force_missing =
                EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_MISSING", "1");

            portal::ensure_portal_available().expect("truthy force available should win");
        }
    }

    #[test]
    fn portal_force_available_falsey_values_do_not_override_force_missing() {
        let lock = GlobalStateLock::new();

        for value in ["0", "false", "no", "off", "", "  ", "enabled"] {
            let _force_available =
                EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_AVAILABLE", value);
            let _force_missing =
                EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_PORTAL_FORCE_MISSING", "1");

            let err = portal::ensure_portal_available()
                .expect_err("falsey value should not force available");
            let message = err.to_string();
            assert!(message.contains("xdg-desktop-portal"), "value={value}");
        }
    }

    #[test]
    fn parse_session_handle_reports_missing_key_and_type_mismatch() {
        let missing = HashMap::new();
        let err = portal::parse_session_handle_from_results(&missing).expect_err("missing key");
        assert_eq!(err, "missing key session_handle");

        let mut wrong_type = HashMap::new();
        wrong_type.insert(
            "session_handle".to_string(),
            zbus::zvariant::OwnedValue::from(7u32),
        );
        let err =
            portal::parse_session_handle_from_results(&wrong_type).expect_err("type mismatch");
        assert_eq!(err, "key session_handle has unexpected type");
    }

    #[test]
    fn parse_streams_reports_missing_key_and_type_mismatch() {
        let missing = HashMap::new();
        let err = portal::parse_streams_from_results(&missing).expect_err("missing key");
        assert_eq!(err, "missing key streams");

        let mut wrong_type = HashMap::new();
        wrong_type.insert(
            "streams".to_string(),
            zbus::zvariant::OwnedValue::from(7u32),
        );
        let err = portal::parse_streams_from_results(&wrong_type).expect_err("type mismatch");
        assert_eq!(err, "key streams has unexpected type");
    }

    #[test]
    fn ensure_response_code_ok_rejects_non_zero_portal_response_code() {
        let err = portal::ensure_response_code_ok(2, HashMap::new())
            .expect_err("non-zero response must be rejected");
        let message = err.to_string();
        assert!(message.contains("portal request failed or was cancelled"));
        assert!(message.contains("response=2"));
    }
}
