//! Display classification must not change the bytes sent to a provider.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn runtime_command_input_bytes_are_unchanged() {
    let contract: Value = serde_json::from_str(include_str!(
        "../../docs/specs/fixtures/history-row-classes-v1.json"
    ))
    .unwrap();
    let mut cases: Vec<(String, String)> = contract["runtime_commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            (
                case["provider"].as_str().unwrap().to_owned(),
                case["runtime_input"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for provider in ["claude", "codex"] {
        cases.push((
            provider.to_owned(),
            "/model\t  keep 🌱 focus  \t".to_owned(),
        ));
    }
    crate::cases::for_each_case(&cases, |(provider, input)| {
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
            input.as_bytes(),
            "{provider}: {input}"
        );
    });
}
