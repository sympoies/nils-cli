use crate::support::*;
use pretty_assertions::assert_eq;

#[test]
fn config_show_and_set_are_secret_free_validated_shell_contracts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let options = base_options(tmp.path())
        .with_env("CLAUDE_CLI_MODEL", "sonnet")
        .with_env("CLAUDE_CLI_EFFORT", "high")
        .with_env("CLAUDE_CLI_AGENT_RUNTIME", "safe")
        .with_env("ANTHROPIC_API_KEY", "must-not-leak")
        .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "also-secret");

    let show = run(&["config", "show"], &options);
    assert_exit(&show, 0);
    assert!(stdout(&show).contains("CLAUDE_CLI_MODEL=sonnet"));
    assert!(stdout(&show).contains("CLAUDE_CLI_EFFORT=high"));
    assert!(stdout(&show).contains("CLAUDE_CLI_AGENT_RUNTIME=safe"));
    assert!(!stdout(&show).contains("must-not-leak"));
    assert!(!stdout(&show).contains("also-secret"));

    let set = run(&["config", "set", "model", "name with ' quote"], &options);
    assert_exit(&set, 0);
    assert_eq!(
        stdout(&set),
        "export CLAUDE_CLI_MODEL='name with '\"'\"' quote'\n"
    );

    let invalid = run(&["config", "set", "effort", "unbounded"], &options);
    assert_exit(&invalid, 64);
    assert!(stderr(&invalid).contains("low|medium|high|xhigh|max"));

    let model_at_limit = "m".repeat(256);
    let accepted = run(
        &["config", "set", "model", &model_at_limit],
        &base_options(tmp.path()),
    );
    assert_exit(&accepted, 0);

    let model_over_limit = "m".repeat(257);
    let rejected = run(
        &["config", "set", "model", &model_over_limit],
        &base_options(tmp.path()),
    );
    assert_exit(&rejected, 64);
    assert!(stderr(&rejected).contains("at most 256 bytes"));

    let invalid_show = run(
        &["config", "show"],
        &base_options(tmp.path()).with_env("CLAUDE_CLI_EFFORT", "unbounded"),
    );
    assert_exit(&invalid_show, 64);
    assert_eq!(stdout(&invalid_show), "");
    assert!(stderr(&invalid_show).contains("low|medium|high|xhigh|max"));
}
