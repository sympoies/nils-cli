use super::*;
use pretty_assertions::assert_eq;

#[test]
fn protected_semver_tags_refuse_before_any_backend_call_even_with_cleanup_or_dry_run() {
    for tag in [
        "v1.2.3",
        "1.2.3",
        "v0.0.0-rc.1+build.07",
        "123.456.789+metadata",
    ] {
        for flags in [vec![], vec!["--cleanup-tag"], vec!["--dry-run"]] {
            let stub = fixture(tag);
            let mut args = vec!["release", "delete", tag];
            args.extend(flags);
            error(&invoke(&stub, &args), "release_version_tag_protected");
            assert!(calls(&stub).is_empty());
        }
    }
}

#[test]
fn release_lifecycle_rejects_unknown_tags_unsafe_selectors_and_wrong_provider_identity() {
    for command in ["view", "edit", "delete"] {
        let stub = fixture("evidence-check");
        let mut args = vec!["release", command, "unknown-tag"];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md"]);
        }
        error(&invoke(&stub, &args), "release_not_found");
        assert!(writes(&stub).is_empty());
        for tag in [
            "../other",
            "refs/tags/v1.2.3",
            "a?b",
            "a#b",
            "a%2Fb",
            "a..b",
            "a b",
            "a@{b",
        ] {
            let stub = fixture(tag);
            args[2] = tag;
            error(&invoke(&stub, &args), "release_tag_invalid");
            assert!(calls(&stub).is_empty());
        }
        for mode in ["wrong-release", "wrong-ref", "network"] {
            let stub = fixture("evidence-check").env("RELEASE_MODE", mode);
            args[2] = "evidence-check";
            assert_ne!(invoke(&stub, &args).code, 0);
            assert!(writes(&stub).is_empty());
        }
    }
}

#[test]
fn lifecycle_dry_runs_emit_explicit_plans_without_identity_or_backend_probes() {
    for command in ["view", "edit", "delete"] {
        let stub = fixture("v1.2.3").env("FORGE_IDENTITY_PRINCIPAL", "missing-fixture");
        let mut args = vec!["release", command, "v1.2.3", "--dry-run"];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md", "--title", TITLE]);
        }
        if command == "delete" {
            args.extend(["--allow-version-tag", "--cleanup-tag"]);
        }
        let d = data(&invoke(&stub, &args));
        assert!(d["plan"].is_array());
        assert!(
            d.to_string()
                .contains("repos/example/widget/releases/tags/v1.2.3")
        );
        if command == "delete" {
            assert!(d.to_string().contains("git/refs/tags/v1.2.3"));
        }
        assert!(!d.to_string().contains(NOTES));
        assert!(calls(&stub).is_empty());
    }
}

#[test]
fn release_effects_and_completions_expose_the_current_lifecycle() {
    for (command, effect, provider) in [
        ("view", "read_only", "network_read"),
        ("edit", "mutation", "network_write"),
        ("delete", "mutation", "network_write"),
    ] {
        let stub = fixture("evidence-check");
        let mut args = vec![
            "operation-effect",
            "--format",
            "json",
            "--",
            "release",
            command,
            "evidence-check",
        ];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md"]);
        }
        let d = data(&run_forge_cli(&stub, &args));
        assert_eq!(d["operation"], format!("release.{command}"));
        assert_eq!(d["effect"], effect);
        assert_eq!(d["provider_effect"], provider);
        assert!(calls(&stub).is_empty());
    }
    for shell in ["bash", "zsh"] {
        let result = run_forge_cli(&fixture("evidence"), &["completion", shell]);
        assert_eq!(result.code, 0);
        for token in ["--allow-version-tag", "--cleanup-tag", "--notes-file"] {
            assert!(result.stdout.contains(token));
        }
    }
}

fn identity_fixture(operations: &str) -> StubEnv {
    let stub = fixture("evidence-check");
    let config = stub.tempdir.path().join("config");
    fs::create_dir_all(config.join("forge-cli")).unwrap();
    fs::write(
        config.join("forge-cli/identity.toml"),
        r#"version=1
[credentials.reader]
kind='env'
name='FIXTURE_RELEASE_CREDENTIAL'
[profiles.reader]
expected_login='fixture-reader'
credential='reader'
commit_name='Example Contributor'
commit_email='reader@example.invalid'
signing_fingerprint='AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
operations=['api_read']
[principals.reader]
profiles=['reader']
[[rules]]
id='reader'
principal='reader'
profile='reader'
repo='github.com/example/widget'
"#
        .replace(
            "operations=['api_read']",
            &format!("operations=[{operations}]"),
        ),
    )
    .unwrap();
    stub.env("XDG_CONFIG_HOME", config.to_string_lossy())
        .env("FORGE_IDENTITY_PRINCIPAL", "reader")
        .env("FIXTURE_RELEASE_CREDENTIAL", "fixture-release-reader")
        .env("GH_TOKEN", "fixture-ambient")
}

#[test]
fn read_identity_can_view_while_edits_and_deletes_require_write_scope() {
    let stub = identity_fixture("'api_read'");
    data(&invoke(&stub, &["release", "view", "evidence-check"]));
    for command in ["edit", "delete"] {
        let mut args = vec!["release", command, "evidence-check"];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md"]);
        }
        error(&invoke(&stub, &args), "identity_operation_denied");
        assert!(writes(&stub).is_empty());
    }
    let audit = fs::read_to_string(
        stub.tempdir
            .path()
            .join("xdg-state/forge-cli/identity-audit.jsonl"),
    )
    .unwrap();
    assert!(audit.contains("api_read"));
    assert!(!calls(&stub).iter().any(|c| c["actor"] == "fixture-ambient"));
}

#[test]
fn lifecycle_rejects_unsupported_providers_and_guarded_payloads_without_backend_calls() {
    for provider in ["gitlab", "local"] {
        let stub = fixture("evidence-check");
        for command in ["view", "edit", "delete"] {
            let mut args = vec![
                "--provider",
                provider,
                "--repo",
                "example/widget",
                "--format",
                "json",
                "release",
                command,
                "evidence-check",
            ];
            if command == "edit" {
                args.extend(["--notes-file", "notes.md"]);
            }
            error(&run_forge_cli(&stub, &args), "provider_unsupported");
        }
        assert!(calls(&stub).is_empty());
    }
    let stub = fixture("evidence-check");
    error(
        &invoke(
            &stub,
            &[
                "release",
                "edit",
                "evidence-check",
                "--notes-file",
                "notes.md",
                "--title",
                "",
            ],
        ),
        "release_title_empty",
    );
    assert!(calls(&stub).is_empty());
}

#[test]
fn lifecycle_writes_bind_selected_identity_and_dry_run_never_probes_credentials() {
    for command in ["edit", "delete"] {
        let stub = identity_fixture("'api_read','api_write'");
        let mut args = vec!["release", command, "evidence-check"];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md"]);
        }
        data(&invoke(&stub, &args));
        assert_eq!(writes(&stub).len(), 1);
        assert!(
            calls(&stub)
                .iter()
                .all(|c| c["actor"] == "fixture-release-reader")
        );
        let audit = fs::read_to_string(
            stub.tempdir
                .path()
                .join("xdg-state/forge-cli/identity-audit.jsonl"),
        )
        .unwrap();
        assert!(audit.contains("api_write"));
        let stub = identity_fixture("'api_read'").env("FIXTURE_RELEASE_CREDENTIAL", "");
        args.push("--dry-run");
        data(&invoke(&stub, &args));
        assert!(calls(&stub).is_empty());
    }
}
