use tempfile::TempDir;

use screen_record::cli::ContainerFormat;
use screen_record::test_mode::TestWriter;

#[test]
fn test_writer_creates_non_empty_file() {
    let dir = TempDir::new().expect("tempdir");
    let output_path = dir.path().join("writer.mov");

    let mut writer = TestWriter::new(&output_path, ContainerFormat::Mov);
    writer.append_frame(b"frame").expect("append frame");
    writer.finish().expect("finish writer");

    let metadata = std::fs::metadata(&output_path).expect("output exists");
    assert!(metadata.len() > 0);
}

#[test]
fn test_writer_finish_without_frame_returns_error() {
    let dir = TempDir::new().expect("tempdir");
    let output_path = dir.path().join("writer.mov");

    let writer = TestWriter::new(&output_path, ContainerFormat::Mov);
    let err = writer
        .finish()
        .expect_err("finish should fail without frames");
    assert_eq!(err.exit_code(), 1);
    assert!(err.to_string().contains("no frames appended"));
}
