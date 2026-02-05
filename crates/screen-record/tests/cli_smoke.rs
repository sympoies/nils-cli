use nils_test_support::bin::resolve;
use nils_test_support::cmd::{run, CmdOutput};

fn screen_record_bin() -> std::path::PathBuf {
    resolve("screen-record")
}

fn run_screen_record(args: &[&str]) -> CmdOutput {
    run(&screen_record_bin(), args, &[], None)
}

#[test]
fn help_includes_key_flags() {
    let out = run_screen_record(&["--help"]);
    assert_eq!(out.code, 0);
    let text = format!("{}{}", out.stdout_text(), out.stderr_text());
    assert!(text.contains("--list-windows"));
    assert!(text.contains("--list-apps"));
    assert!(text.contains("--audio"));
    assert!(text.contains("--duration"));
}

#[test]
fn invalid_flag_exits_nonzero() {
    let out = run_screen_record(&["--definitely-not-a-flag"]);
    assert_ne!(out.code, 0);
}
