//! Display classification must not change the bytes sent to a provider.
use super::*;
use pretty_assertions::assert_eq;

/// Each send runs the real binary against its own fixture directory, so the
/// cases are independent. Running them in small batches keeps the test close to
/// one send long instead of 38.
const CONCURRENT_SENDS: usize = 4;

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
    for batch in cases.chunks(CONCURRENT_SENDS) {
        std::thread::scope(|scope| {
            let sends: Vec<_> = batch
                .iter()
                .map(|(provider, input)| {
                    scope.spawn(move || {
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
                    })
                })
                .collect();
            // Join every send before reporting, so one failure does not hide
            // another, and name the failing case.
            let failed: Vec<_> = sends
                .into_iter()
                .zip(batch)
                .filter_map(|(send, (provider, input))| {
                    send.join().err().map(|_| format!("{provider}: {input:?}"))
                })
                .collect();
            assert!(failed.is_empty(), "cases failed: {failed:#?}");
        });
    }
}
