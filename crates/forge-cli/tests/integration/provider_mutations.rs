//! Repository-bound GitHub writes, with hermetic backend and identity fixtures.
use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::assert_eq;
use std::fs;

const STUB: &str = r#"#!/bin/sh
actor=unselected
case "$GH_TOKEN" in
 fixture-a) actor=account-a;;
 fixture-b) actor=account-b;;
esac
if test "$1:$2" = api:user; then printf '{"login":"%s"}' "$actor"; exit 0; fi
printf '%s\n' "$actor" >> "$CALL_LOG"
printf '%s\n' "$@" >> "$ARGV_LOG"
previous=''
for arg in "$@"; do
  if test "$previous" = --input; then
    test -f "$arg" || exit 9
    cat "$arg" > "$PAYLOAD_LOG"
  fi
  previous="$arg"
done
case "$1:$2" in
 release:create)
 previous=''
 for arg in "$@"; do
   if test "$previous" = --notes-file; then
     test -f "$arg" || exit 9
     wc -c < "$arg" > "$NOTES_SIZE_LOG"
   fi
   previous="$arg"
 done
 printf 'https://github.com/example/widget/releases/tag/v1.0.0\n';;
 release:upload|workflow:run) :;;
 api:*) case "$*" in
 *DELETE*|*POST*) :;;
 *) python3 - "$PAYLOAD_LOG" <<'FIXTURE_PY'
import json, sys
payload = json.load(open(sys.argv[1]))
print(json.dumps({"id":17,"html_url":"https://github.com/example/widget/issues/1#issuecomment-17","body":payload["body"]}, ensure_ascii=False))
FIXTURE_PY
 ;;
 esac;;
 *) exit 9;;
esac
"#;

fn fixture() -> StubEnv {
    let stub = StubEnv::new().gh_stub(STUB);
    fs::write(stub.tempdir.path().join("notes.md"), "Release notes\n").unwrap();
    fs::write(stub.tempdir.path().join("body.md"), "Updated comment").unwrap();
    fs::write(stub.tempdir.path().join("asset.bin"), "asset fixture").unwrap();
    let calls = stub.tempdir.path().join("calls");
    let argv = stub.tempdir.path().join("argv");
    let notes_size = stub.tempdir.path().join("notes-size");
    let payload = stub.tempdir.path().join("payload.json");
    stub.env("CALL_LOG", calls.to_string_lossy())
        .env("ARGV_LOG", argv.to_string_lossy())
        .env("NOTES_SIZE_LOG", notes_size.to_string_lossy())
        .env("PAYLOAD_LOG", payload.to_string_lossy())
}
fn cases() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "release.create",
            vec![
                "release",
                "create",
                "v1.0.0",
                "--title",
                "Release 1.0",
                "--notes-file",
                "notes.md",
                "--verify-tag",
            ],
        ),
        (
            "release.upload",
            vec!["release", "upload", "v1.0.0", "asset.bin", "--clobber"],
        ),
        (
            "workflow.dispatch",
            vec![
                "workflow",
                "dispatch",
                "build.yml",
                "--ref",
                "main",
                "--input",
                "mode=check",
            ],
        ),
        (
            "comment.edit",
            vec!["comment", "edit", "17", "--body-file", "body.md"],
        ),
        ("comment.delete", vec!["comment", "delete", "17"]),
    ]
}
fn invoke(stub: &StubEnv, args: &[&str]) -> super::support::CmdOutput {
    let mut full = vec![
        "--provider",
        "github",
        "--repo",
        "example/widget",
        "--format",
        "json",
    ];
    full.extend_from_slice(args);
    run_forge_cli(stub, &full)
}

#[test]
fn provider_mutations_emit_typed_success_and_explicit_repository_plans() {
    for (op, args) in cases() {
        let stub = fixture();
        let result = invoke(&stub, &args);
        assert_eq!(result.code, 0, "{op}: {} {}", result.stdout, result.stderr);
        let envelope = parse_envelope(&result.stdout);
        assert_eq!(envelope["schema_version"], format!("cli.forge-cli.{op}.v1"));
        assert_eq!(envelope["data"]["provider"], "github");
        assert_eq!(envelope["data"]["repository"], "example/widget");
        let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
        assert!(argv.contains("example/widget"), "{op}: {argv}");
        if op == "comment.edit" {
            assert!(argv.contains("repos/example/widget/issues/comments/17\n--method\nPATCH"));
            assert_eq!(envelope["data"]["body"], "Updated comment");
        }
        if op == "comment.delete" {
            assert!(argv.contains("repos/example/widget/issues/comments/17\n--method\nDELETE"));
            assert_eq!(envelope["data"]["deleted"], true);
        }
        if op == "workflow.dispatch" {
            assert!(argv.contains("repos/example/widget/actions/workflows/build.yml/dispatches"));
            let payload: serde_json::Value = serde_json::from_slice(
                &fs::read(stub.tempdir.path().join("payload.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(payload["inputs"]["mode"], "check");
            assert_eq!(payload["ref"], "main");
            assert_eq!(envelope["data"]["dispatched"], true);
        }
    }
}

#[test]
fn provider_mutations_dry_run_never_runs_backend() {
    for (op, mut args) in cases() {
        let stub = fixture();
        args.push("--dry-run");
        let result = invoke(&stub, &args);
        assert_eq!(result.code, 0, "{op}: {}", result.stdout);
        assert!(parse_envelope(&result.stdout)["data"]["plan"].is_array());
        assert!(!stub.tempdir.path().join("calls").exists());
    }
}

#[test]
fn provider_mutations_are_named_network_writes() {
    for (op, args) in cases() {
        let stub = fixture();
        let mut argv = vec![
            "operation-effect",
            "--format",
            "json",
            "--",
            "--repo",
            "example/widget",
        ];
        argv.extend_from_slice(&args);
        let result = run_forge_cli(&stub, &argv);
        assert_eq!(result.code, 0, "{op}: {}", result.stdout);
        let data = parse_envelope(&result.stdout)["data"].clone();
        assert_eq!(data["operation"], op);
        assert_eq!(data["effect"], "mutation");
        assert_eq!(data["provider_effect"], "network_write");
    }
}

const POLICY: &str = r#"version=1
[credentials.a]
kind='env'
name='FIXTURE_WRITE_A'
[credentials.b]
kind='env'
name='FIXTURE_WRITE_B'
[profiles.a]
expected_login='account-a'
credential='a'
commit_name='Example Contributor A'
commit_email='a@example.invalid'
signing_fingerprint='AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
operations=['api_read','api_write']
[profiles.b]
expected_login='account-b'
credential='b'
commit_name='Example Contributor B'
commit_email='b@example.invalid'
signing_fingerprint='BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB'
operations=['api_read','api_write']
[principals.writer-a]
profiles=['a']
[principals.writer-b]
profiles=['b']
[[rules]]
id='a'
principal='writer-a'
profile='a'
repo='github.com/example/widget'
[[rules]]
id='b'
principal='writer-b'
profile='b'
repo='github.com/example/widget'
"#;
fn identity_fixture(principal: &str, policy: &str) -> StubEnv {
    let stub = fixture();
    let config = stub.tempdir.path().join("config");
    fs::create_dir_all(config.join("forge-cli")).unwrap();
    fs::write(config.join("forge-cli/identity.toml"), policy).unwrap();
    stub.env("XDG_CONFIG_HOME", config.to_string_lossy())
        .env("FORGE_IDENTITY_PRINCIPAL", principal)
        .env("FIXTURE_WRITE_A", "fixture-a")
        .env("FIXTURE_WRITE_B", "fixture-b")
        .env("GH_TOKEN", "ambient-fixture-token")
}

#[test]
fn provider_mutations_bind_two_principals_in_parallel_without_ambient_fallback() {
    std::thread::scope(|scope| {
        for (principal, actor) in [("writer-a", "account-a"), ("writer-b", "account-b")] {
            scope.spawn(move || {
                for (op, args) in cases() {
                    let stub = identity_fixture(principal, POLICY);
                    let result = invoke(&stub, &args);
                    assert_eq!(
                        result.code, 0,
                        "{principal} {op}: {} {}",
                        result.stdout, result.stderr
                    );
                    assert_eq!(
                        fs::read_to_string(stub.tempdir.path().join("calls")).unwrap(),
                        format!("{actor}\n")
                    );
                    assert!(!result.stdout.contains("fixture-a"));
                    assert!(!result.stdout.contains("fixture-b"));
                    let audit = fs::read_to_string(
                        stub.tempdir
                            .path()
                            .join("xdg-state/forge-cli/identity-audit.jsonl"),
                    )
                    .unwrap();
                    assert!(audit.contains(principal));
                    assert!(audit.contains("api_write"));
                }
            });
        }
    });
}

#[test]
fn provider_mutations_fail_closed_for_denied_write_scope() {
    for (op, args) in cases() {
        let policy = POLICY.replace("'api_read','api_write'", "'api_read'");
        let stub = identity_fixture("writer-a", &policy);
        let result = invoke(&stub, &args);
        assert_eq!(result.code, 65, "{op}: {} {}", result.stdout, result.stderr);
        assert!(
            result.stdout.contains("identity_operation_denied"),
            "{}",
            result.stdout
        );
        assert!(!stub.tempdir.path().join("calls").exists());
    }
}

#[test]
fn provider_mutations_reject_bad_inputs_and_unsupported_providers_before_writing() {
    for (args, error) in [
        (
            vec![
                "workflow",
                "dispatch",
                "build.yml",
                "--ref",
                "main",
                "--input",
                "missing-equals",
            ],
            "workflow_input_invalid",
        ),
        (
            vec![
                "workflow",
                "dispatch",
                "build.yml",
                "--ref",
                "main",
                "--input",
                "mode=a",
                "--input",
                "mode=b",
            ],
            "workflow_input_duplicate",
        ),
        (
            vec!["comment", "edit", "17", "--body", ""],
            "body_missing_summary",
        ),
        (
            vec!["release", "upload", "v1.0.0", "missing.bin"],
            "asset_unreadable",
        ),
    ] {
        let stub = fixture();
        let result = invoke(&stub, &args);
        assert_eq!(result.code, 65, "{} {}", result.stdout, result.stderr);
        assert!(result.stdout.contains(error), "{}", result.stdout);
        assert!(!stub.tempdir.path().join("calls").exists());
    }
    for (_, args) in cases() {
        let stub = fixture();
        let mut full = vec![
            "--provider",
            "gitlab",
            "--repo",
            "example/widget",
            "--format",
            "json",
        ];
        full.extend_from_slice(&args);
        let result = run_forge_cli(&stub, &full);
        assert!(
            result.stdout.contains("provider_unsupported"),
            "{}",
            result.stdout
        );
        assert!(!stub.tempdir.path().join("calls").exists());
    }
}

#[test]
fn provider_mutations_bind_enterprise_authority_and_review_comment_route() {
    let stub = fixture();
    let result = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--host",
            "github.example.invalid",
            "--repo",
            "example/widget",
            "--format",
            "json",
            "comment",
            "delete",
            "17",
            "--kind",
            "review",
        ],
    );
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
    assert!(argv.contains("--hostname\ngithub.example.invalid"));
    assert!(argv.contains("repos/example/widget/pulls/comments/17"));
}

#[test]
fn provider_mutations_surface_backend_failures_and_missing_identity_credentials() {
    for (op, args) in cases() {
        let stub = fixture().gh_stub("#!/bin/sh\necho rejected >&2\nexit 1\n");
        let result = invoke(&stub, &args);
        assert_eq!(result.code, 1, "{op}: {} {}", result.stdout, result.stderr);
        assert_eq!(parse_envelope(&result.stdout)["ok"], false);
        let stub = identity_fixture(
            "writer-a",
            &POLICY.replace("FIXTURE_WRITE_A", "FIXTURE_MISSING_CREDENTIAL"),
        );
        let result = invoke(&stub, &args);
        assert_eq!(result.code, 65, "{op}: {} {}", result.stdout, result.stderr);
        assert!(result.stdout.contains("identity_credential_missing"));
        assert!(!stub.tempdir.path().join("calls").exists());
    }
}

#[test]
fn provider_mutations_forward_release_flags_and_literal_workflow_values() {
    let stub = fixture();
    let result = invoke(
        &stub,
        &[
            "release",
            "create",
            "v1.0.0",
            "--title",
            "Release 1.0",
            "--notes-file",
            "notes.md",
            "--target",
            "main",
            "--draft",
            "--prerelease",
            "--latest",
            "false",
            "asset.bin",
        ],
    );
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
    for flag in [
        "--target\nmain",
        "--draft",
        "--prerelease",
        "--latest=false",
        "--notes-file",
    ] {
        assert!(argv.contains(flag), "missing {flag}: {argv}");
    }
    assert_eq!(parse_envelope(&result.stdout)["data"]["assets"], 1);
    let stub = fixture();
    let result = invoke(
        &stub,
        &[
            "workflow",
            "dispatch",
            "42",
            "--ref",
            "v1.0.0",
            "--input",
            "value=@literal=a",
            "--input",
            "empty=",
        ],
    );
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
    assert!(argv.contains("repos/example/widget/actions/workflows/42/dispatches"));
    let payload: serde_json::Value =
        serde_json::from_slice(&fs::read(stub.tempdir.path().join("payload.json")).unwrap())
            .unwrap();
    assert_eq!(payload["inputs"]["value"], "@literal=a");
    assert_eq!(payload["inputs"]["empty"], "");
}

#[test]
fn provider_mutations_guard_posted_text_and_reject_mismatched_edit_response() {
    let stub = fixture();
    let result = invoke(
        &stub,
        &[
            "comment",
            "edit",
            "17",
            "--body",
            "Generated with Claude Code",
        ],
    );
    assert_eq!(result.code, 65, "{}", result.stdout);
    assert!(result.stdout.contains("agent_attribution_present"));
    assert!(!stub.tempdir.path().join("calls").exists());
    let stub = fixture().gh_stub("#!/bin/sh\nprintf '{\"id\":18,\"html_url\":\"https://github.com/example/widget/issues/1#issuecomment-18\",\"body\":\"Updated comment\"}'\n");
    let result = invoke(&stub, &["comment", "edit", "17", "--body-file", "body.md"]);
    assert_eq!(result.code, 65, "{}", result.stdout);
    assert!(result.stdout.contains("comment_response_mismatch"));
    let stub = fixture().gh_stub("#!/bin/sh\nprintf malformed\n");
    let result = invoke(&stub, &["comment", "edit", "17", "--body-file", "body.md"]);
    assert_eq!(result.code, 70, "{}", result.stdout);
}

#[test]
fn provider_mutations_large_release_notes_use_file_transport_and_compact_plan() {
    let stub = fixture();
    fs::write(stub.tempdir.path().join("notes.md"), "n".repeat(200_000)).unwrap();
    let args = [
        "release",
        "create",
        "v1.0.0",
        "--title",
        "Release 1.0",
        "--notes-file",
        "notes.md",
    ];
    let result = invoke(&stub, &args);
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
    assert!(argv.contains("--notes-file\n"));
    assert!(argv.len() < 4096, "notes must not be expanded into argv");
    assert_eq!(
        fs::read_to_string(stub.tempdir.path().join("notes-size"))
            .unwrap()
            .trim(),
        "200000"
    );
    let backend_args: Vec<_> = argv.lines().collect();
    let notes_path = backend_args
        .windows(2)
        .find(|pair| pair[0] == "--notes-file")
        .unwrap()[1];
    assert!(
        !std::path::Path::new(notes_path).exists(),
        "validated transport file must be cleaned up"
    );
    let mut dry_args = args.to_vec();
    dry_args.push("--dry-run");
    let result = invoke(&stub, &dry_args);
    assert_eq!(result.code, 0, "{}", result.stdout);
    assert!(
        result.stdout.len() < 4096,
        "plan must not include the notes payload"
    );
}

#[test]
fn provider_mutations_release_stdin_notes_are_validated_and_file_backed() {
    let stub = fixture();
    let result = super::support::run_forge_cli_with_stdin(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "example/widget",
            "--format",
            "json",
            "release",
            "create",
            "v1.0.0",
            "--title",
            "Release 1.0",
            "--notes-file",
            "-",
        ],
        "Release notes from stdin",
    );
    assert_eq!(result.code, 0, "{} {}", result.stdout, result.stderr);
    assert_eq!(
        fs::read_to_string(stub.tempdir.path().join("notes-size"))
            .unwrap()
            .trim(),
        "24"
    );
    let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
    assert!(argv.contains("--notes-file\n"));
    assert!(!argv.contains("Release notes from stdin"));
}

#[test]
fn provider_mutations_large_comment_and_workflow_inputs_use_compact_file_plans() {
    let large = "界".repeat(50_000);
    for command in ["comment", "workflow"] {
        let stub = fixture();
        fs::write(stub.tempdir.path().join("body.md"), &large).unwrap();
        fs::write(
            stub.tempdir.path().join("inputs.json"),
            serde_json::json!({"value": large}).to_string(),
        )
        .unwrap();
        let args = if command == "comment" {
            vec!["comment", "edit", "17", "--body-file", "body.md"]
        } else {
            vec![
                "workflow",
                "dispatch",
                "build.yml",
                "--ref",
                "main",
                "--inputs-file",
                "inputs.json",
            ]
        };
        let result = invoke(&stub, &args);
        assert_eq!(
            result.code, 0,
            "{command}: {} {}",
            result.stdout, result.stderr
        );
        let argv = fs::read_to_string(stub.tempdir.path().join("argv")).unwrap();
        assert!(
            argv.len() < 4096,
            "{command} payload must not be expanded into argv"
        );
        let payload: serde_json::Value =
            serde_json::from_slice(&fs::read(stub.tempdir.path().join("payload.json")).unwrap())
                .unwrap();
        assert_eq!(
            if command == "comment" {
                &payload["body"]
            } else {
                &payload["inputs"]["value"]
            },
            &serde_json::Value::String(large.clone())
        );
        let backend_args: Vec<_> = argv.lines().collect();
        let payload_path = backend_args
            .windows(2)
            .find(|pair| pair[0] == "--input")
            .unwrap()[1];
        assert!(!std::path::Path::new(payload_path).exists());
        let mut dry_args = args.clone();
        dry_args.push("--dry-run");
        let result = invoke(&stub, &dry_args);
        assert_eq!(result.code, 0, "{}", result.stdout);
        assert!(
            result.stdout.len() < 4096,
            "{command} dry-run payload must stay file-backed"
        );
    }
}

#[test]
fn provider_mutations_workflow_input_file_rejects_duplicate_keys_and_non_strings() {
    for json in [r#"{"mode":"a","mode":"b"}"#, r#"{"mode":true}"#, r#"[]"#] {
        let stub = fixture();
        fs::write(stub.tempdir.path().join("inputs.json"), json).unwrap();
        let result = invoke(
            &stub,
            &[
                "workflow",
                "dispatch",
                "build.yml",
                "--ref",
                "main",
                "--inputs-file",
                "inputs.json",
            ],
        );
        assert_eq!(result.code, 65, "{}", result.stdout);
        assert!(result.stdout.contains("workflow_inputs_file_invalid"));
        assert!(!stub.tempdir.path().join("calls").exists());
    }
}

#[test]
fn provider_mutations_workflow_file_keys_are_validated_without_inline_reparsing() {
    let stub = fixture();
    fs::write(
        stub.tempdir.path().join("inputs.json"),
        r#"{"mode=unexpected":"value"}"#,
    )
    .unwrap();
    let result = invoke(
        &stub,
        &[
            "workflow",
            "dispatch",
            "build.yml",
            "--ref",
            "main",
            "--inputs-file",
            "inputs.json",
        ],
    );
    assert_eq!(result.code, 65, "{}", result.stdout);
    assert!(result.stdout.contains("workflow_input_invalid"));
    assert!(!stub.tempdir.path().join("calls").exists());
}
