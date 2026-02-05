use screen_record::error::CliError;

#[test]
fn runtime_error_prefixes_message() {
    let err = CliError::runtime("boom");
    assert_eq!(err.exit_code(), 1);
    assert_eq!(err.to_string(), "error: boom");
}
