//! Display classification must not change the bytes sent to a provider.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn runtime_command_input_bytes_are_unchanged() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../docs/specs/fixtures/history-row-classes-v1.json"
    ))
    .unwrap();
    for case in contract["runtime_commands"].as_array().unwrap() {
        let provider = case["provider"].as_str().unwrap();
        let input = case["runtime_input"].as_str().unwrap();
        let fixture = Fixture::new(provider);
        let output = fixture.send_text(Some(input), &[]);
        assert_eq!(
            output.code,
            0,
            "{provider}: {input}: {}",
            output.stderr_text()
        );
        assert_eq!(
            fs::read(fixture.root().join("buffer")).unwrap(),
            input.as_bytes()
        );
    }
    for provider in ["claude", "codex"] {
        let input = "/model\t  keep 🌱 focus  \t";
        let fixture = Fixture::new(provider);
        let output = fixture.send_text(Some(input), &[]);
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        assert_eq!(
            fs::read(fixture.root().join("buffer")).unwrap(),
            input.as_bytes()
        );
    }
}
