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
