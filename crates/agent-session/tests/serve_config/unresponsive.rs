use super::*;
use pretty_assertions::assert_eq;

#[test]
fn unresponsive_bound_file_environment_precedence_and_invalid_values() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\nunresponsive_seconds = 7\n",
    );
    let output = fixture.check(&config, &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(envelope(&output)["data"]["unresponsive"]["source"], "file");
    let output = fixture.check(&config, &[("AGENT_SESSION_UNRESPONSIVE_SECONDS", "9")]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = envelope(&output);
    assert_eq!(value["data"]["unresponsive"]["source"], "environment");
    assert_eq!(warnings(&value).len(), 1);
    assert!(warnings(&value)[0].contains("AGENT_SESSION_UNRESPONSIVE_SECONDS"));
    for bad in ["0", "", " ", "-1", "1.5", "many", "18446744073709551616"] {
        let output = fixture.check(&config, &[("AGENT_SESSION_UNRESPONSIVE_SECONDS", bad)]);
        assert_eq!(
            failure(&fixture, &output)["code"],
            "serve-config-invalid-value"
        );
    }
    for bad in ["0", "-1", "1.5", "\"10\"", "true"] {
        let config = fixture.write(
            "invalid.toml",
            &format!(
                "schema_version = \"agent-session.serve-config.v1\"\nunresponsive_seconds = {bad}\n"
            ),
        );
        assert_eq!(
            failure(&fixture, &fixture.check(&config, &[]))["code"],
            "serve-config-invalid-value"
        );
    }
}
