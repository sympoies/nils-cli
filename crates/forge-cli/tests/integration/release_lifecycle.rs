//! Release lifecycle acceptance at the CLI/provider boundary, with no live writes.
use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli, run_forge_cli_with_stdin};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;

mod guards;

const NOTES: &str = "## Reviewed notes\n\nLiteral `code`, $(echo unchanged), @value, café.\n";
const TITLE: &str = "Reviewed $(echo literal) `title`";

fn fixture(tag: &str) -> StubEnv {
    let stub = StubEnv::new().gh_stub(include_str!("release_lifecycle/provider.py"));
    let state = json!({
        "release_exists": true, "tag_exists": true, "mutated": false,
        "release": {"id":17, "tag_name":tag, "target_commitish":"main",
          "html_url":format!("https://github.com/example/widget/releases/tag/{tag}"),
          "name":"Original title", "draft":false, "prerelease":true,
          "body":"Original notes", "assets":[
            {"name":"artifact.zip", "size":42, "digest":"sha256:provider-value"},
            {"name":"notes.txt", "size":7}
          ]}
    });
    fs::write(stub.tempdir.path().join("state.json"), state.to_string()).unwrap();
    fs::write(stub.tempdir.path().join("notes.md"), NOTES).unwrap();
    let root = stub.tempdir.path().to_string_lossy().into_owned();
    stub.env("RELEASE_FIXTURE", root)
}
fn argv<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut full = vec![
        "--provider",
        "github",
        "--repo",
        "example/widget",
        "--format",
        "json",
    ];
    full.extend_from_slice(args);
    full
}
fn invoke(stub: &StubEnv, args: &[&str]) -> CmdOutput {
    run_forge_cli(stub, &argv(args))
}
fn data(result: &CmdOutput) -> Value {
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    parse_envelope(&result.stdout)["data"].clone()
}
fn calls(stub: &StubEnv) -> Vec<Value> {
    fs::read_to_string(stub.tempdir.path().join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn writes(stub: &StubEnv) -> Vec<Value> {
    calls(stub)
        .into_iter()
        .filter(|c| {
            c["argv"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| matches!(v.as_str(), Some("PATCH" | "DELETE")))
        })
        .collect()
}
fn error(result: &CmdOutput, expected: &str) -> Value {
    assert_ne!(result.code, 0, "{}", result.stdout);
    let envelope = parse_envelope(&result.stdout);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], expected, "{}", result.stdout);
    envelope["error"].clone()
}

#[test]
fn release_view_reports_identity_resolved_commit_and_optional_provider_digest() {
    for mode in ["", "annotated"] {
        let stub = fixture("evidence/check").env("RELEASE_MODE", mode);
        let result = invoke(&stub, &["release", "view", "evidence/check"]);
        let d = data(&result);
        assert_eq!(
            parse_envelope(&result.stdout)["schema_version"],
            "cli.forge-cli.release.view.v1"
        );
        assert_eq!(d["release_id"], 17);
        assert_eq!(d["tag"], "evidence/check");
        assert_eq!(d["target_commit"], "a".repeat(40));
        assert_eq!(d["target_commitish"], "main");
        assert_eq!(d["draft"], false);
        assert_eq!(d["prerelease"], true);
        assert_eq!(d["notes"], "Original notes");
        assert_eq!(
            d["assets"],
            json!([
                {"name":"artifact.zip","size":42,"digest":"sha256:provider-value"},
                {"name":"notes.txt","size":7,"digest":null}
            ])
        );
        assert!(writes(&stub).is_empty());
        let result = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "example/widget",
                "release",
                "view",
                "evidence/check",
            ],
        );
        assert_eq!(result.code, 0, "{}", result.stderr);
        for expected in [
            "evidence/check",
            "17",
            "Original notes",
            "artifact.zip",
            "42",
            "sha256:provider-value",
            &"a".repeat(40),
            "draft",
            "prerelease",
        ] {
            assert!(
                result.stdout.contains(expected),
                "{expected}: {}",
                result.stdout
            );
        }
    }
}

#[test]
fn release_edit_transports_file_and_stdin_literally_and_verifies_preserved_state() {
    for stdin in [false, true] {
        let stub = fixture("evidence-notes");
        let args = argv(&[
            "release",
            "edit",
            "evidence-notes",
            "--notes-file",
            if stdin { "-" } else { "notes.md" },
            "--title",
            TITLE,
        ]);
        let result = if stdin {
            run_forge_cli_with_stdin(&stub, &args, NOTES)
        } else {
            run_forge_cli(&stub, &args)
        };
        let d = data(&result);
        assert_eq!(d["notes"], NOTES);
        assert_eq!(d["title"], TITLE);
        assert_eq!(d["draft"], false);
        assert_eq!(d["prerelease"], true);
        assert_eq!(d["target_commitish"], "main");
        let payload: Value =
            serde_json::from_slice(&fs::read(stub.tempdir.path().join("payload.json")).unwrap())
                .unwrap();
        assert_eq!(payload, json!({"body":NOTES,"name":TITLE}));
        assert_eq!(writes(&stub).len(), 1);
        assert!(
            !calls(&stub)
                .iter()
                .any(|c| c["argv"].as_array().unwrap().iter().any(|v| v == NOTES))
        );
    }
    let stub = fixture("evidence-notes");
    let d = data(&invoke(
        &stub,
        &[
            "release",
            "edit",
            "evidence-notes",
            "--notes-file",
            "notes.md",
        ],
    ));
    assert_eq!(d["title"], "Original title");
}

#[test]
fn release_delete_verifies_release_absence_and_only_explicit_tag_cleanup() {
    for (tag, allow) in [("evidence-check", false), ("v1.2.3", true)] {
        for cleanup in [false, true] {
            let stub = fixture(tag);
            let mut args = vec!["release", "delete", tag];
            if allow {
                args.push("--allow-version-tag");
            }
            if cleanup {
                args.push("--cleanup-tag");
            }
            let d = data(&invoke(&stub, &args));
            assert_eq!(d["release_deleted"], true);
            assert_eq!(d["tag_deleted"], cleanup);
            assert_eq!(d["release_id"], 17);
            assert_eq!(writes(&stub).len(), if cleanup { 2 } else { 1 });
            let state: Value =
                serde_json::from_slice(&fs::read(stub.tempdir.path().join("state.json")).unwrap())
                    .unwrap();
            assert_eq!(state["tag_exists"], !cleanup);
        }
    }
}

#[test]
fn release_lifecycle_preserves_typed_partial_and_readback_failures() {
    for (mode, command, expected) in [
        ("edit-mismatch", "edit", "release_readback_mismatch"),
        ("edit-state-changed", "edit", "release_readback_mismatch"),
        ("readback-fails", "edit", "release_readback_failed"),
        ("delete-rejected", "delete", "release_delete_failed"),
        ("release-remains", "delete", "release_readback_mismatch"),
        ("release-recreated", "delete", "release_readback_mismatch"),
        ("readback-fails", "delete", "release_readback_failed"),
        ("tag-rejected", "delete", "release_cleanup_failed"),
        ("tag-remains", "delete", "release_readback_mismatch"),
    ] {
        let stub = fixture("evidence-check").env("RELEASE_MODE", mode);
        let mut args = vec!["release", command, "evidence-check"];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md"]);
        } else {
            args.push("--cleanup-tag");
        }
        let e = error(&invoke(&stub, &args), expected);
        assert!(e["details"]["next_action"].is_string(), "{mode}: {e}");
        if mode.starts_with("tag-") {
            assert_eq!(e["details"]["release_deleted"], true);
            assert_eq!(e["details"]["tag_deleted"], false);
        }
        assert!(writes(&stub).len() <= 2);
    }
}

#[test]
fn release_edit_refuses_changed_tag_at_readback() {
    let stub = fixture("evidence-check").env("RELEASE_MODE", "tag-moved");
    let result = invoke(
        &stub,
        &[
            "release",
            "edit",
            "evidence-check",
            "--notes-file",
            "notes.md",
        ],
    );
    error(&result, "release_readback_mismatch");
    assert_eq!(writes(&stub).len(), 1);
}

#[test]
fn release_cleanup_preserves_tag_that_moved_after_release_deletion() {
    let stub = fixture("evidence-check").env("RELEASE_MODE", "tag-moved");
    let result = invoke(
        &stub,
        &["release", "delete", "evidence-check", "--cleanup-tag"],
    );
    let e = error(&result, "release_cleanup_failed");
    assert_eq!(e["details"]["release_deleted"], true);
    assert_eq!(writes(&stub).len(), 1);
}

#[test]
fn draft_without_tag_resolves_actual_target_commit_without_inventing_a_sha() {
    let stub = fixture("evidence-draft");
    let path = stub.tempdir.path().join("state.json");
    let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["release"]["draft"] = true.into();
    state["tag_exists"] = false.into();
    fs::write(&path, state.to_string()).unwrap();
    let d = data(&invoke(&stub, &["release", "view", "evidence-draft"]));
    assert_eq!(d["draft"], true);
    assert_eq!(d["target_commit"], "c".repeat(40));
    assert!(writes(&stub).is_empty());
}

#[test]
fn draft_edit_and_delete_use_resolved_release_identity() {
    for command in ["edit", "delete"] {
        let stub = fixture("evidence-draft");
        let path = stub.tempdir.path().join("state.json");
        let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        state["release"]["draft"] = true.into();
        state["tag_exists"] = false.into();
        fs::write(&path, state.to_string()).unwrap();
        let mut args = vec!["release", command, "evidence-draft"];
        if command == "edit" {
            args.extend(["--notes-file", "notes.md"]);
        } else {
            args.push("--cleanup-tag");
        }
        let d = data(&invoke(&stub, &args));
        if command == "edit" {
            assert_eq!(d["draft"], true);
        } else {
            assert_eq!(d["release_deleted"], true);
            assert_eq!(d["tag_deleted"], true);
        }
        assert_eq!(writes(&stub).len(), 1);
    }
}
