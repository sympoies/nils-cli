mod common;

#[test]
fn version_flag_exits_zero() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let out = common::run_image_processing(dir.path(), &["--version"], &[]);

    assert_eq!(out.code, 0);
    assert!(
        out.stdout.contains("image-processing"),
        "stdout: {}",
        out.stdout
    );
}
