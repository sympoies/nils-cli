#[cfg(target_os = "linux")]
mod linux_request_permission {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use nils_test_support::bin::resolve;
    use nils_test_support::cmd::{run_with, CmdOptions};
    use tempfile::TempDir;

    #[test]
    fn request_permission_missing_ffmpeg_reports_install_hint() {
        let bin = resolve("screen-record");
        let empty_path = TempDir::new().expect("tempdir");

        let options = CmdOptions::new()
            .with_env_remove("CODEX_SCREEN_RECORD_TEST_MODE")
            .with_env_remove("DISPLAY")
            .with_env_remove("WAYLAND_DISPLAY")
            .with_env("PATH", &empty_path.path().to_string_lossy());

        let out = run_with(&bin, &["--request-permission"], &options);
        assert_eq!(out.code, 1);
        let stderr = out.stderr_text();
        assert!(stderr.contains("ffmpeg"));
        assert!(stderr.contains("apt-get install ffmpeg"));
    }

    #[test]
    fn request_permission_wayland_requires_xorg() {
        let bin = resolve("screen-record");
        let temp_dir = TempDir::new().expect("tempdir");
        let ffmpeg_path = temp_dir.path().join("ffmpeg");

        fs::write(&ffmpeg_path, "#!/bin/sh\nexit 0\n").expect("write ffmpeg stub");
        let mut perms = fs::metadata(&ffmpeg_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&ffmpeg_path, perms).expect("chmod ffmpeg stub");

        let options = CmdOptions::new()
            .with_env_remove("CODEX_SCREEN_RECORD_TEST_MODE")
            .with_env_remove("DISPLAY")
            .with_env("WAYLAND_DISPLAY", "wayland-0")
            .with_env("PATH", &temp_dir.path().to_string_lossy());

        let out = run_with(&bin, &["--request-permission"], &options);
        assert_eq!(out.code, 1);
        let stderr = out.stderr_text();
        assert!(stderr.contains("DISPLAY"));
        assert!(stderr.contains("Xorg"));
    }
}
