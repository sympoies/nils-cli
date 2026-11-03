//! Executable command text is independent of the permanent schema namespace.

pub(crate) const SCHEMA_PREFIX: &str = "agent-session";

/// Use the shared build-time name so command hints agree with helper lookup.
#[cfg(not(test))]
pub(crate) const fn executable() -> &'static str {
    nils_common::session_binary::NAME
}

// Keep unit-test command identities thread-scoped; integration tests exercise
// the shared build-time name in both default and configured builds.
#[cfg(test)]
thread_local! {
    static NAME: std::cell::Cell<&'static str> = const { std::cell::Cell::new(nils_common::session_binary::NAME) };
}

#[cfg(test)]
pub(crate) fn executable() -> &'static str {
    NAME.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn with_name<T>(name: &'static str, action: impl FnOnce() -> T) -> T {
    struct Restore(&'static str);
    impl Drop for Restore {
        fn drop(&mut self) {
            NAME.with(|name| name.set(self.0));
        }
    }
    let _restore = Restore(NAME.with(|current| current.replace(name)));
    action()
}

pub(crate) fn dsh_readiness_guidance() -> String {
    format!(
        "The launch profile's own launcher installs the bridge hook for dsh panes; use {} readiness for the authenticated runtime checkpoint contract",
        executable()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn renamed_command_identity_reaches_broker_workspace_and_lifecycle_recovery() {
        with_name("session-control", || {
            assert!(
                crate::coordination::unauthorized()
                    .into_inner()
                    .hint
                    .unwrap()
                    .contains("`session-control broker status`")
            );
            let error = crate::origin::invalid_workspace().into_inner();
            assert_eq!(
                error.details.unwrap()["recovery"]["command"],
                "session-control start --agent codex --workspace example-workspace"
            );
            let exit = json!({
                "code": 130, "signal": null, "reason": "provider-exited",
                "unexpected": true, "observed_at": "2030-01-01T00:00:00Z",
                "recovery": {}
            });
            let projection =
                crate::lifecycle::recovery::board_projection(Some(&exit), "worker").unwrap();
            assert_eq!(
                projection["recovery"]["command"],
                "session-control resume worker"
            );
            assert!(dsh_readiness_guidance().contains("session-control readiness"));
        });
        assert_eq!(executable(), nils_common::session_binary::NAME);
        assert_eq!(SCHEMA_PREFIX, "agent-session");
    }
}
