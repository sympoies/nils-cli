//! Generation context belongs to the received snapshot, never to inferred file times.
use crate::support::*;
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;

fn annotation() -> Value {
    json!({
        "schema_version":"nils.auth-generation.v1", "provider":"claude", "account":"alpha",
        "version":{"authority_id":"11111111-1111-4111-8111-111111111111",
            "authority_epoch":"22222222-2222-4222-8222-222222222222", "generation":"2"},
        "snapshot_id":"33333333-3333-4333-8333-333333333333",
        "authority_committed_at":"2030-01-01T00:00:00.000Z", "issued_at":null,
        "expires_at":"2100-01-01T00:00:00.000Z",
        "auth":{"revision":"1", "state":"valid", "reason":"provider_accepted",
            "source":"provider_refresh", "observed_at":"2030-01-01T00:00:00.000Z"}
    })
}

fn profile(generation: Value) -> Value {
    json!({"profile":"alpha", "credential_generation":generation,
        "claudeAiOauth":{"accessToken":"fixture-access", "refreshToken":"fixture-refresh",
            "expiresAt":4102444800000_i64},
        "oauthAccount":{"accountUuid":"fixture-account", "organizationUuid":"fixture-org"}})
}

fn pull(
    payload: Value,
    all: bool,
    name: &str,
    root: &std::path::Path,
) -> nils_test_support::cmd::CmdOutput {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let source = root.join("export.json");
    std::fs::write(&source, payload.to_string()).unwrap();
    let ssh = bin.join("ssh");
    std::fs::write(&ssh, "#!/bin/sh\ncat \"$GENERATION_FIXTURE\"\n").unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let options = base_options(root)
        .with_path_prepend(&bin)
        .with_env("GENERATION_FIXTURE", &path_str(&source))
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off");
    let mut args = vec![
        "auth",
        "remote",
        "pull",
        "--ssh",
        "authority",
        "--access-only",
        "--keychain",
        "off",
        "--format",
        "json",
    ];
    let into = path_str(&root.join("accounts"));
    if all {
        args.extend(["--all", "--into", &into]);
    } else if name.is_empty() {
        args.extend(["--current", "--write-active"]);
    } else {
        args.extend(["--name", name, "--write-active"]);
    }
    run(&args, &options)
}

#[test]
fn generation_context_survives_single_and_all_writes_without_secret_fields() {
    for (all, name) in [(false, "alpha"), (false, ""), (true, "")] {
        let tmp = tempfile::tempdir().unwrap();
        let entry = profile(annotation());
        let payload = if all {
            json!({"current":"alpha", "profiles":[entry]})
        } else {
            entry
        };
        let output = pull(payload, all, name, tmp.path());
        assert_exit(&output, 0);
        let result: Value = serde_json::from_str(&stdout(&output)).unwrap();
        let written = if all {
            &result["result"]["profiles"][0]
        } else {
            &result["result"]
        };
        assert_eq!(written["credential_generation"], annotation());
        assert!(!stdout(&output).contains("fixture-access"));
        assert!(!stdout(&output).contains("fixture-refresh"));
        let credentials = if all {
            tmp.path().join("accounts/alpha/.credentials.json")
        } else {
            tmp.path().join("claude-config/.credentials.json")
        };
        assert!(
            !std::fs::read_to_string(credentials)
                .unwrap()
                .contains("credential_generation")
        );
    }
}

#[test]
fn generation_invalid_or_mismatched_metadata_refuses_before_any_store_write() {
    for (field, replacement) in [
        ("provider", json!("codex")),
        ("account", json!("beta")),
        ("snapshot_id", json!("invalid")),
        ("issued_at", json!("2030-02-30T00:00:00.000Z")),
        ("unexpected", json!("fixture-secret")),
    ] {
        for all in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let mut invalid = annotation();
            invalid[field] = replacement.clone();
            // An invalid later entry must also prevent the first entry's write.
            let payload = if all {
                json!({"current":"alpha", "profiles":[profile(annotation()), profile(invalid)]})
            } else {
                profile(invalid)
            };
            let output = pull(payload, all, "alpha", tmp.path());
            assert_ne!(output.code, 0, "{field}: {}", stdout(&output));
            let error: Value = serde_json::from_str(&stdout(&output)).unwrap();
            assert_eq!(
                error["error"]["code"], "remote-export-invalid-generation",
                "{field}"
            );
            assert!(!tmp.path().join("accounts").exists());
            assert!(!tmp.path().join("claude-config/.credentials.json").exists());
            assert!(!stdout(&output).contains("fixture-secret"));
        }
    }
}

#[test]
fn generation_named_pull_cannot_publish_another_accounts_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let output = pull(profile(annotation()), false, "beta", tmp.path());
    assert_ne!(output.code, 0);
    let error: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(error["error"]["code"], "remote-export-invalid-generation");
}

#[test]
fn generation_nested_contract_is_strict_and_nullable_fields_are_required() {
    for (pointer, replacement) in [
        ("/version/generation", json!("0")),
        ("/version/generation", json!("02")),
        ("/version/generation", json!(2)),
        ("/version/generation", json!("18446744073709551616")),
        (
            "/version/authority_id",
            json!("11111111-1111-1111-8111-111111111111"),
        ),
        ("/auth/revision", json!("01")),
        ("/auth/state", json!("healthy")),
        ("/auth/reason", json!("provider_revoked")),
        ("/auth/observed_at", Value::Null),
        ("/authority_committed_at", json!("2030-01-01T00:00:00Z")),
        ("/authority_committed_at", json!("2030-01-01T00:00:60.000Z")),
        ("/expires_at", json!("2030-01-01T00:00:00.000+00:00")),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let mut g = annotation();
        *g.pointer_mut(pointer).unwrap() = replacement;
        let output = pull(profile(g), false, "alpha", tmp.path());
        assert_ne!(output.code, 0, "{pointer}");
        assert!(!tmp.path().join("claude-config/.credentials.json").exists());
    }
    for pointer in ["", "/version", "/auth"] {
        let tmp = tempfile::tempdir().unwrap();
        let mut g = annotation();
        g.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("token".into(), json!("fixture-secret"));
        let output = pull(profile(g), false, "alpha", tmp.path());
        assert_ne!(output.code, 0);
        assert!(!stdout(&output).contains("fixture-secret"));
    }
    for field in ["issued_at", "expires_at"] {
        let tmp = tempfile::tempdir().unwrap();
        let mut g = annotation();
        g.as_object_mut().unwrap().remove(field);
        assert_ne!(pull(profile(g), false, "alpha", tmp.path()).code, 0);
    }
}

#[test]
fn generation_context_does_not_turn_a_partial_account_write_into_publication() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("accounts/beta")).unwrap(); // unowned destination
    std::fs::write(tmp.path().join("accounts/beta/retained"), "retained").unwrap();
    let mut beta = profile(annotation());
    beta["profile"] = json!("beta");
    beta["credential_generation"]["account"] = json!("beta");
    let output = pull(
        json!({"current":"alpha", "profiles":[profile(annotation()),beta]}),
        true,
        "alpha",
        tmp.path(),
    );
    assert_exit(&output, 1);
    let body: Value = serde_json::from_str(&stdout(&output)).unwrap();
    let profiles = body["result"]["profiles"].as_array().unwrap();
    assert_eq!(profiles[0]["written"], true);
    assert_eq!(profiles[1]["written"], false);
    assert_eq!(profiles[1]["credential_generation"]["account"], "beta");
    assert!(profiles[1].get("published").is_none());
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("accounts/beta/retained")).unwrap(),
        "retained"
    );
}

#[test]
fn generation_accepts_maximum_counter_and_unknown_provider_observation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut g = annotation();
    g["version"]["generation"] = json!("18446744073709551615");
    g["auth"] = json!({"revision":"18446744073709551615", "state":"unknown",
        "reason":"unvalidated", "source":"unknown", "observed_at":null});
    let output = pull(profile(g.clone()), false, "alpha", tmp.path());
    assert_exit(&output, 0);
    let body: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(body["result"]["credential_generation"], g);
}
