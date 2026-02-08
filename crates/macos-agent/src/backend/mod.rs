pub mod applescript;
pub mod cliclick;
pub mod hammerspoon;
pub mod input_source;
pub mod process;

use crate::backend::hammerspoon::HammerspoonAxBackend;
use crate::backend::process::ProcessRunner;
use crate::error::CliError;
use crate::model::{
    AxActionPerformRequest, AxActionPerformResult, AxAttrGetRequest, AxAttrGetResult,
    AxAttrSetRequest, AxAttrSetResult, AxClickRequest, AxClickResult, AxListRequest, AxListResult,
    AxSessionListResult, AxSessionStartRequest, AxSessionStartResult, AxSessionStopRequest,
    AxSessionStopResult, AxTypeRequest, AxTypeResult, AxWatchPollRequest, AxWatchPollResult,
    AxWatchStartRequest, AxWatchStartResult, AxWatchStopRequest, AxWatchStopResult,
};
use crate::test_mode;

pub trait AxBackendAdapter {
    fn list(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxListRequest,
        timeout_ms: u64,
    ) -> Result<AxListResult, CliError>;

    fn click(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxClickRequest,
        timeout_ms: u64,
    ) -> Result<AxClickResult, CliError>;

    fn type_text(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxTypeRequest,
        timeout_ms: u64,
    ) -> Result<AxTypeResult, CliError>;

    fn attr_get(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxAttrGetRequest,
        _timeout_ms: u64,
    ) -> Result<AxAttrGetResult, CliError> {
        Err(CliError::runtime(
            "AX attribute get is not supported by this backend",
        ))
    }

    fn attr_set(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxAttrSetRequest,
        _timeout_ms: u64,
    ) -> Result<AxAttrSetResult, CliError> {
        Err(CliError::runtime(
            "AX attribute set is not supported by this backend",
        ))
    }

    fn action_perform(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxActionPerformRequest,
        _timeout_ms: u64,
    ) -> Result<AxActionPerformResult, CliError> {
        Err(CliError::runtime(
            "AX action perform is not supported by this backend",
        ))
    }

    fn session_start(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxSessionStartRequest,
        _timeout_ms: u64,
    ) -> Result<AxSessionStartResult, CliError> {
        Err(CliError::runtime(
            "AX session start is not supported by this backend",
        ))
    }

    fn session_list(
        &self,
        _runner: &dyn ProcessRunner,
        _timeout_ms: u64,
    ) -> Result<AxSessionListResult, CliError> {
        Err(CliError::runtime(
            "AX session list is not supported by this backend",
        ))
    }

    fn session_stop(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxSessionStopRequest,
        _timeout_ms: u64,
    ) -> Result<AxSessionStopResult, CliError> {
        Err(CliError::runtime(
            "AX session stop is not supported by this backend",
        ))
    }

    fn watch_start(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxWatchStartRequest,
        _timeout_ms: u64,
    ) -> Result<AxWatchStartResult, CliError> {
        Err(CliError::runtime(
            "AX watch start is not supported by this backend",
        ))
    }

    fn watch_poll(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxWatchPollRequest,
        _timeout_ms: u64,
    ) -> Result<AxWatchPollResult, CliError> {
        Err(CliError::runtime(
            "AX watch poll is not supported by this backend",
        ))
    }

    fn watch_stop(
        &self,
        _runner: &dyn ProcessRunner,
        _request: &AxWatchStopRequest,
        _timeout_ms: u64,
    ) -> Result<AxWatchStopResult, CliError> {
        Err(CliError::runtime(
            "AX watch stop is not supported by this backend",
        ))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AppleScriptAxBackend;

impl AxBackendAdapter for AppleScriptAxBackend {
    fn list(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxListRequest,
        timeout_ms: u64,
    ) -> Result<AxListResult, CliError> {
        applescript::ax_list(runner, request, timeout_ms)
    }

    fn click(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxClickRequest,
        timeout_ms: u64,
    ) -> Result<AxClickResult, CliError> {
        applescript::ax_click(runner, request, timeout_ms)
    }

    fn type_text(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxTypeRequest,
        timeout_ms: u64,
    ) -> Result<AxTypeResult, CliError> {
        applescript::ax_type(runner, request, timeout_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxBackendPreference {
    Auto,
    Hammerspoon,
    AppleScript,
}

impl AxBackendPreference {
    pub fn resolve() -> Self {
        if let Ok(raw) = std::env::var("CODEX_MACOS_AGENT_AX_BACKEND") {
            match raw.trim().to_ascii_lowercase().as_str() {
                "hammerspoon" | "hs" => return Self::Hammerspoon,
                "applescript" | "jxa" => return Self::AppleScript,
                "auto" => return Self::Auto,
                _ => {}
            }
        }

        if test_mode::enabled() {
            // Tests rely on deterministic osascript stubs.
            Self::AppleScript
        } else {
            Self::Auto
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AutoAxBackend {
    preference: AxBackendPreference,
}

impl Default for AutoAxBackend {
    fn default() -> Self {
        Self {
            preference: AxBackendPreference::resolve(),
        }
    }
}

impl AutoAxBackend {
    fn fallback_with_hint(primary_error: CliError, fallback_error: CliError) -> CliError {
        fallback_error.with_hint(format!(
            "Hammerspoon backend failed first: {}",
            primary_error.message()
        ))
    }
}

impl AxBackendAdapter for AutoAxBackend {
    fn list(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxListRequest,
        timeout_ms: u64,
    ) -> Result<AxListResult, CliError> {
        match self.preference {
            AxBackendPreference::Hammerspoon => {
                HammerspoonAxBackend.list(runner, request, timeout_ms)
            }
            AxBackendPreference::AppleScript => {
                AppleScriptAxBackend.list(runner, request, timeout_ms)
            }
            AxBackendPreference::Auto => match HammerspoonAxBackend
                .list(runner, request, timeout_ms)
            {
                Ok(result) => Ok(result),
                Err(primary_error) if hammerspoon::is_backend_unavailable_error(&primary_error) => {
                    AppleScriptAxBackend
                        .list(runner, request, timeout_ms)
                        .map_err(|fallback_error| {
                            Self::fallback_with_hint(primary_error, fallback_error)
                        })
                }
                Err(error) => Err(error),
            },
        }
    }

    fn click(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxClickRequest,
        timeout_ms: u64,
    ) -> Result<AxClickResult, CliError> {
        match self.preference {
            AxBackendPreference::Hammerspoon => {
                HammerspoonAxBackend.click(runner, request, timeout_ms)
            }
            AxBackendPreference::AppleScript => {
                AppleScriptAxBackend.click(runner, request, timeout_ms)
            }
            AxBackendPreference::Auto => match HammerspoonAxBackend
                .click(runner, request, timeout_ms)
            {
                Ok(result) => Ok(result),
                Err(primary_error) if hammerspoon::is_backend_unavailable_error(&primary_error) => {
                    AppleScriptAxBackend
                        .click(runner, request, timeout_ms)
                        .map_err(|fallback_error| {
                            Self::fallback_with_hint(primary_error, fallback_error)
                        })
                }
                Err(error) => Err(error),
            },
        }
    }

    fn type_text(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxTypeRequest,
        timeout_ms: u64,
    ) -> Result<AxTypeResult, CliError> {
        match self.preference {
            AxBackendPreference::Hammerspoon => {
                HammerspoonAxBackend.type_text(runner, request, timeout_ms)
            }
            AxBackendPreference::AppleScript => {
                AppleScriptAxBackend.type_text(runner, request, timeout_ms)
            }
            AxBackendPreference::Auto => {
                match HammerspoonAxBackend.type_text(runner, request, timeout_ms) {
                    Ok(result) => Ok(result),
                    Err(primary_error)
                        if hammerspoon::is_backend_unavailable_error(&primary_error) =>
                    {
                        AppleScriptAxBackend
                            .type_text(runner, request, timeout_ms)
                            .map_err(|fallback_error| {
                                Self::fallback_with_hint(primary_error, fallback_error)
                            })
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    fn attr_get(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxAttrGetRequest,
        timeout_ms: u64,
    ) -> Result<AxAttrGetResult, CliError> {
        HammerspoonAxBackend.attr_get(runner, request, timeout_ms)
    }

    fn attr_set(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxAttrSetRequest,
        timeout_ms: u64,
    ) -> Result<AxAttrSetResult, CliError> {
        HammerspoonAxBackend.attr_set(runner, request, timeout_ms)
    }

    fn action_perform(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxActionPerformRequest,
        timeout_ms: u64,
    ) -> Result<AxActionPerformResult, CliError> {
        HammerspoonAxBackend.action_perform(runner, request, timeout_ms)
    }

    fn session_start(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxSessionStartRequest,
        timeout_ms: u64,
    ) -> Result<AxSessionStartResult, CliError> {
        HammerspoonAxBackend.session_start(runner, request, timeout_ms)
    }

    fn session_list(
        &self,
        runner: &dyn ProcessRunner,
        timeout_ms: u64,
    ) -> Result<AxSessionListResult, CliError> {
        HammerspoonAxBackend.session_list(runner, timeout_ms)
    }

    fn session_stop(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxSessionStopRequest,
        timeout_ms: u64,
    ) -> Result<AxSessionStopResult, CliError> {
        HammerspoonAxBackend.session_stop(runner, request, timeout_ms)
    }

    fn watch_start(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxWatchStartRequest,
        timeout_ms: u64,
    ) -> Result<AxWatchStartResult, CliError> {
        HammerspoonAxBackend.watch_start(runner, request, timeout_ms)
    }

    fn watch_poll(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxWatchPollRequest,
        timeout_ms: u64,
    ) -> Result<AxWatchPollResult, CliError> {
        HammerspoonAxBackend.watch_poll(runner, request, timeout_ms)
    }

    fn watch_stop(
        &self,
        runner: &dyn ProcessRunner,
        request: &AxWatchStopRequest,
        timeout_ms: u64,
    ) -> Result<AxWatchStopResult, CliError> {
        HammerspoonAxBackend.watch_stop(runner, request, timeout_ms)
    }
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    use crate::backend::AxBackendPreference;

    #[test]
    fn backend_preference_defaults_to_applescript_in_test_mode() {
        let lock = GlobalStateLock::new();
        let _test_mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _backend = EnvGuard::remove(&lock, "CODEX_MACOS_AGENT_AX_BACKEND");
        assert_eq!(
            AxBackendPreference::resolve(),
            AxBackendPreference::AppleScript
        );
    }

    #[test]
    fn backend_preference_env_overrides_test_mode_default() {
        let lock = GlobalStateLock::new();
        let _test_mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _backend = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_AX_BACKEND", "hammerspoon");
        assert_eq!(
            AxBackendPreference::resolve(),
            AxBackendPreference::Hammerspoon
        );
    }
}
