use crate::backend::process::{ProcessRequest, ProcessRunner, map_failure};
use crate::error::CliError;
use crate::test_mode;

const TEST_INPUT_SOURCE_CURRENT_ENV: &str = "AGENTS_MACOS_AGENT_TEST_INPUT_SOURCE_CURRENT";
const DEFAULT_ABC_SOURCE: &str = "com.apple.keylayout.ABC";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSourceSwitchState {
    pub previous: String,
    pub current: String,
    pub switched: bool,
}

pub fn current(runner: &dyn ProcessRunner, timeout_ms: u64) -> Result<String, CliError> {
    if test_mode::enabled() {
        return Ok(std::env::var(TEST_INPUT_SOURCE_CURRENT_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ABC_SOURCE.to_string()));
    }

    let request = ProcessRequest::new("im-select", Vec::new(), timeout_ms.max(1));
    runner
        .run(&request)
        .map(|output| normalize_stdout(&output.stdout))
        .map_err(|failure| {
            map_failure("input-source.current", failure)
                .with_hint("Install `im-select` via Homebrew: brew install im-select")
        })
}

pub fn switch(
    runner: &dyn ProcessRunner,
    source_id: &str,
    timeout_ms: u64,
) -> Result<InputSourceSwitchState, CliError> {
    let target = normalize_input_source_token(source_id);
    if target.trim().is_empty() {
        return Err(CliError::usage("--id cannot be empty").with_operation("input-source.switch"));
    }

    let previous = current(runner, timeout_ms)?;
    if test_mode::enabled() {
        return Ok(InputSourceSwitchState {
            switched: !previous.eq_ignore_ascii_case(&target),
            previous,
            current: target,
        });
    }

    let request = ProcessRequest::new("im-select", vec![target.clone()], timeout_ms.max(1));
    runner.run(&request).map_err(|failure| {
        map_failure("input-source.switch", failure)
            .with_hint("Install `im-select` via Homebrew: brew install im-select")
    })?;

    let current = current(runner, timeout_ms)?;
    Ok(InputSourceSwitchState {
        switched: !previous.eq_ignore_ascii_case(&current),
        previous,
        current,
    })
}

pub fn normalize_input_source_token(raw: &str) -> String {
    let trimmed = raw.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "abc" | "english" | "us" | "u.s." => DEFAULT_ABC_SOURCE.to_string(),
        _ => trimmed.to_string(),
    }
}

fn normalize_stdout(raw: &str) -> String {
    raw.trim().to_string()
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    use crate::backend::process::{ProcessFailure, ProcessOutput, ProcessRequest, ProcessRunner};

    use super::{current, normalize_input_source_token, switch};

    struct FixedRunner {
        stdout: String,
    }

    impl FixedRunner {
        fn new(stdout: impl Into<String>) -> Self {
            Self {
                stdout: stdout.into(),
            }
        }
    }

    impl ProcessRunner for FixedRunner {
        fn run(&self, _request: &ProcessRequest) -> Result<ProcessOutput, ProcessFailure> {
            Ok(ProcessOutput {
                stdout: self.stdout.clone(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn normalize_token_maps_common_aliases() {
        assert_eq!(
            normalize_input_source_token("abc"),
            "com.apple.keylayout.ABC"
        );
        assert_eq!(
            normalize_input_source_token("US"),
            "com.apple.keylayout.ABC"
        );
    }

    #[test]
    fn normalize_token_preserves_case_for_full_source_id() {
        assert_eq!(
            normalize_input_source_token("com.apple.keylayout.ABC"),
            "com.apple.keylayout.ABC"
        );
    }

    #[test]
    fn current_uses_test_mode_env_when_enabled() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_MODE", "1");
        let _value = EnvGuard::set(
            &lock,
            "AGENTS_MACOS_AGENT_TEST_INPUT_SOURCE_CURRENT",
            "com.apple.keylayout.US",
        );
        let out = current(&FixedRunner::new("ignored"), 100).expect("test mode current");
        assert_eq!(out, "com.apple.keylayout.US");
    }

    #[test]
    fn switch_returns_simulated_state_in_test_mode() {
        let lock = GlobalStateLock::new();
        let _mode = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_MODE", "1");
        let _value = EnvGuard::set(
            &lock,
            "AGENTS_MACOS_AGENT_TEST_INPUT_SOURCE_CURRENT",
            "com.apple.keylayout.US",
        );
        let state = switch(&FixedRunner::new("ignored"), "abc", 100).expect("switch");
        assert!(state.switched);
        assert_eq!(state.previous, "com.apple.keylayout.US");
        assert_eq!(state.current, "com.apple.keylayout.ABC");
    }
}
