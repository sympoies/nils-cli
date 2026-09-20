use crate::support::*;

#[test]
fn main_no_args_prints_help_and_exits_zero() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = run(&[], &base_options(tmp.path()));
    assert_exit(&output, 0);
    let help = stdout(&output);
    assert!(help.contains("claude-cli"));
    assert!(help.contains("Authentication command group"));
    assert!(help.contains("Configuration command group"));
    assert!(help.contains("Prompt-segment command group"));
    assert!(help.contains("completion"));
}

#[test]
fn main_unknown_command_exits_64() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output = run(&["not-a-real-command"], &base_options(tmp.path()));
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("unrecognized subcommand"));
}

#[test]
fn main_completion_exports_bash_and_zsh_scripts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let options = base_options(tmp.path());

    let zsh = run(&["completion", "zsh"], &options);
    assert_exit(&zsh, 0);
    let zsh_text = stdout(&zsh);
    assert!(zsh_text.contains("#compdef claude-cli"));
    assert!(zsh_text.contains("auth:Authentication command group"));
    assert!(zsh_text.contains("config:Configuration command group"));
    assert!(zsh_text.contains("prompt-segment:Prompt-segment command group"));
    assert!(zsh_text.contains(":shell -- Shell to generate completion script for:(bash zsh)"));

    let bash = run(&["completion", "bash"], &options);
    assert_exit(&bash, 0);
    let bash_text = stdout(&bash);
    assert!(bash_text.contains("_claude__cli()"));
    assert!(bash_text.contains("complete -F _claude__cli"));
    assert!(bash_text.contains("opts=\"-h --help bash zsh\""));
}
