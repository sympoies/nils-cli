use super::*;
use pretty_assertions::assert_eq;

#[test]
fn stale_draft_bound_zero_environment_precedence_and_invalid_values() {
    let fixture = Fixture::new();
    let config = fixture.write(
        "serve.toml",
        "schema_version = \"agent-session.serve-config.v1\"\nstale_draft_seconds = 0\n",
    );
    let output = fixture.check(&config, &[]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(envelope(&output)["data"]["stale_draft"]["source"], "file");
    let output = fixture.check(&config, &[("AGENT_SESSION_STALE_DRAFT_SECONDS", "300")]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let value = envelope(&output);
    assert_eq!(value["data"]["stale_draft"]["source"], "environment");
    assert_eq!(warnings(&value).len(), 1);
    assert!(warnings(&value)[0].contains("AGENT_SESSION_STALE_DRAFT_SECONDS"));
    for bad in ["", " ", "-1", "1.5", "many", "18446744073709551616"] {
        let output = fixture.check(&config, &[("AGENT_SESSION_STALE_DRAFT_SECONDS", bad)]);
        assert_eq!(
            failure(&fixture, &output)["code"],
            "serve-config-invalid-value"
        );
    }
    for bad in ["-1", "1.5", "\"10\"", "true"] {
        let config = fixture.write(
            "invalid.toml",
            &format!(
                "schema_version = \"agent-session.serve-config.v1\"\nstale_draft_seconds = {bad}\n"
            ),
        );
        assert_eq!(
            failure(&fixture, &fixture.check(&config, &[]))["code"],
            "serve-config-invalid-value"
        );
    }
}
