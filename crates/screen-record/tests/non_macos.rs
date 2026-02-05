#[cfg(not(target_os = "macos"))]
mod non_macos {
    use nils_test_support::bin::resolve;
    use nils_test_support::cmd::{run_with, CmdOptions};

    #[test]
    fn exits_with_usage_error_without_test_mode() {
        let bin = resolve("screen-record");
        let options = CmdOptions::new().with_env_remove("CODEX_SCREEN_RECORD_TEST_MODE");
        let out = run_with(&bin, &["--list-windows"], &options);
        assert_eq!(out.code, 2);
        assert!(out.stderr_text().contains("only supported on macOS"));
    }
}
