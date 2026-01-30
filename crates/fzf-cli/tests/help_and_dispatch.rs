mod common;

#[test]
fn help_prints_usage_and_commands() {
    let temp = tempfile::TempDir::new().unwrap();
    let out = common::run_fzf_cli(temp.path(), &["help"], &[], None);
    assert_eq!(out.code, 0);
    assert!(
        out.stdout.contains("Usage: fzf-cli <command> [args]"),
        "missing usage: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Commands:"),
        "missing Commands header: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("git-commit"),
        "missing command in help: {}",
        out.stdout
    );
}

#[test]
fn unknown_command_prints_message_and_exits_1() {
    let temp = tempfile::TempDir::new().unwrap();
    let out = common::run_fzf_cli(temp.path(), &["nope"], &[], None);
    assert_eq!(out.code, 1);
    assert!(
        out.stdout.contains("❗ Unknown command: nope"),
        "missing unknown command line: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Run 'fzf-cli help' for usage."),
        "missing usage hint: {}",
        out.stdout
    );
}
