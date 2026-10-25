//! Body-free, exact-target maintainer redaction of GitHub timeline comments.
use super::write;
use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::{BINARY, CommentRedactArgs, GlobalFlags};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use base64::{Engine, engine::general_purpose::STANDARD};
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::time::Duration;

const OP: &str = "comment.redact";
// Transport the body privately in an encoded field so credential scrubbing in
// the shared runner cannot change the bytes used for the freshness digest.
// Neither provider stdout nor stderr is ever emitted by this operation.
const SNAPSHOT_QUERY: &str = r#"if (.body|type) == "string" then {id,user,issue_url,body_base64:(.body|@base64)} else error("invalid comment body") end"#;

#[derive(Deserialize)]
struct Actor {
    login: String,
}
#[derive(Deserialize)]
struct Permissions {
    push: bool,
}
#[derive(Deserialize)]
struct Repository {
    full_name: String,
    permissions: Permissions,
}
#[derive(Deserialize)]
struct Snapshot {
    id: u64,
    user: Actor,
    issue_url: String,
    body_base64: String,
}
#[derive(Serialize)]
struct Payload<'a> {
    provider: &'static str,
    repository: &'a str,
    id: u64,
    author: &'a str,
    body_sha256: String,
    status: &'static str,
    history_redacted: bool,
}

fn invalid(kind: &'static str, message: &str) -> ForgeError {
    write::invalid(OP, kind, message)
}
fn read_failed() -> ForgeError {
    ForgeError::unavailable(
        schema_version_for(BINARY, OP, 1),
        "comment_redaction_read_failed",
        "unable to verify comment redaction metadata; no provider content is displayed",
        None,
    )
}
fn call(ctx: &ProviderContext, endpoint: &str) -> BackendCall {
    let mut argv: Vec<OsString> = vec!["api".into()];
    ctx.push_github_api_hostname(&mut argv);
    argv.push(endpoint.into());
    BackendCall::new(BackendProgram::Gh, argv)
}
fn read<T: DeserializeOwned>(call: &BackendCall) -> Result<T, ForgeError> {
    let output = crate::rate_limit::default_runner()
        .run_with_timeout(call, Some(Duration::from_secs(30)))
        .map_err(|error| {
            // Identity preparation has not read a comment response. Preserve
            // its typed policy/credential failures; backend diagnostics may
            // contain provider content and must stay private.
            if error.kind().starts_with("identity_") {
                return error;
            }
            match error {
                ForgeError::BackendError { .. } | ForgeError::BackendUnavailable { .. } => {
                    read_failed()
                }
                error => error,
            }
        })?;
    serde_json::from_str(&output.stdout).map_err(|_| read_failed())
}
fn digest(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn snapshot(
    ctx: &ProviderContext,
    endpoint: &str,
    repo: &str,
    id: u64,
) -> Result<(Snapshot, Vec<u8>), ForgeError> {
    let mut request = call(ctx, endpoint);
    request.argv.extend(["--jq".into(), SNAPSHOT_QUERY.into()]);
    let current: Snapshot = read(&request)?;
    let origin = if ctx.host == "github.com" {
        "https://api.github.com".to_string()
    } else {
        format!("https://{}/api/v3", ctx.host)
    };
    let prefix = format!("{origin}/repos/{repo}/issues/").to_ascii_lowercase();
    let issue_url = current.issue_url.to_ascii_lowercase();
    let valid_issue = issue_url
        .strip_prefix(&prefix)
        .is_some_and(|number| number.parse::<u64>().is_ok_and(|n| n > 0));
    if current.id != id || !valid_issue {
        return Err(invalid(
            "comment_response_mismatch",
            "provider returned a comment outside the exact requested target",
        ));
    }
    if !valid_login(&current.user.login) {
        return Err(invalid(
            "comment_snapshot_invalid",
            "comment author login is missing or invalid",
        ));
    }
    let body = STANDARD
        .decode(&current.body_base64)
        .map_err(|_| read_failed())?;
    std::str::from_utf8(&body).map_err(|_| read_failed())?;
    Ok((current, body))
}
fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'[' | b']'))
}
fn protected_record(body: &[u8]) -> bool {
    let body = String::from_utf8_lossy(body);
    [
        "<!-- forge-cli:review-",
        "<!-- forge-cli:thread-disposition:",
    ]
    .iter()
    .any(|marker| body.contains(marker))
}
fn unresolved() -> ForgeError {
    ForgeError::unavailable_with_details(
        schema_version_for(BINARY, OP, 1),
        "comment_redaction_unresolved",
        "replacement was not verified; do not replay blindly; inspect the exact comment and provider permissions",
        serde_json::json!({
            "retryable": false,
            "next_action": "inspect_comment_and_permissions",
            "history_redacted": false
        }),
    )
}

pub(super) fn run(
    global: &GlobalFlags,
    args: CommentRedactArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if global.repo.is_none() {
        return Err(invalid(
            "repository_required",
            "redaction requires an explicit --repo",
        ));
    }
    let ctx = write::target(global, OP)?;
    let repo = ctx.repo.as_deref().expect("explicit repository");
    let endpoint = format!("repos/{repo}/issues/comments/{}", args.id);
    let replacement = if args.inspect {
        None
    } else {
        let expected = args
            .expected_body_sha256
            .as_deref()
            .expect("required digest");
        if expected.len() != 64
            || !expected
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid(
                "comment_digest_invalid",
                "expected digest must be 64 lowercase SHA-256 hex digits",
            ));
        }
        let author = args.expected_author.as_deref().expect("required author");
        if !valid_login(author) {
            return Err(invalid(
                "comment_author_invalid",
                "expected author must be a non-empty login",
            ));
        }
        let path = args.body_file.as_deref().expect("required body file");
        if path == "-" {
            return Err(invalid(
                "body_file_required",
                "redaction requires a replacement body file",
            ));
        }
        let body = write::text_file(path, OP)?;
        if body.trim().is_empty() {
            return Err(invalid(
                "body_missing_summary",
                "replacement body must not be empty",
            ));
        }
        write::guard_text(&body, "replacement comment")?;
        if protected_record(body.as_bytes()) {
            return Err(invalid(
                "comment_redaction_protected",
                "review ledger and handoff records require their owning workflow",
            ));
        }
        Some(body)
    };
    if global.dry_run {
        let mut planned = call(&ctx, &endpoint);
        if args.inspect {
            planned.argv.extend(["--jq".into(), SNAPSHOT_QUERY.into()]);
        } else {
            planned.argv.extend([
                "--method".into(),
                "PATCH".into(),
                "--input".into(),
                "<payload-file>".into(),
            ]);
        }
        let plan = planned.plan_argv();
        let text_plan = plan.join(" ");
        return Ok(emit_success(
            schema_version_for(BINARY, OP, 1),
            serde_json::json!({"provider":"github", "repository":repo, "id":args.id,
                "status":"planned", "history_redacted":false, "plan":plan}),
            format,
            |_| println!("would run: {text_plan}; author, permission and digest are unverified"),
        ));
    }
    // All reads and the PATCH inherit the enclosing invocation's selected
    // principal. The provider still enforces token permissions on the PATCH.
    let actor: Actor = read(&call(&ctx, "user"))?;
    let repository: Repository = read(&call(&ctx, &format!("repos/{repo}")))?;
    if !valid_login(&actor.login) || !repository.full_name.eq_ignore_ascii_case(repo) {
        return Err(invalid(
            "comment_response_mismatch",
            "unable to verify the repository and authenticated actor",
        ));
    }
    if !repository.permissions.push {
        return Err(invalid(
            "comment_redaction_forbidden",
            "repository write permission is required; request an authorized maintainer",
        ));
    }
    let (current, body) = snapshot(&ctx, &endpoint, repo, args.id)?;
    let status = if let Some(replacement) = &replacement {
        let author = args.expected_author.as_deref().expect("required author");
        // Machine-owned review records must keep their immutable history.
        if protected_record(&body) {
            return Err(invalid(
                "comment_redaction_protected",
                "review ledger and handoff records require their owning workflow",
            ));
        }
        if !current.user.login.eq_ignore_ascii_case(author) {
            return Err(invalid(
                "comment_author_mismatch",
                "comment author differs from the expected author",
            ));
        }
        if body == replacement.as_bytes() {
            "already_replaced"
        } else {
            if digest(&body) != *args.expected_body_sha256.as_ref().expect("required digest") {
                return Err(invalid(
                    "comment_stale",
                    "comment body differs from the expected digest; inspect before retrying",
                ));
            }
            let payload = write::payload_file(
                serde_json::json!({"body":replacement})
                    .to_string()
                    .as_bytes(),
                OP,
            )?;
            let mut patch = call(&ctx, &endpoint);
            patch.argv.extend([
                "--method".into(),
                "PATCH".into(),
                "--input".into(),
                payload.path().as_os_str().to_owned(),
            ]);
            // REST calls are not retried by the shared runner. Even a successful
            // PATCH response is insufficient: verify with a separate fresh GET.
            let _result = crate::rate_limit::default_runner()
                .run_with_timeout(&patch, Some(Duration::from_secs(30)));
            let (after, body) =
                snapshot(&ctx, &endpoint, repo, args.id).map_err(|_| unresolved())?;
            if !after.user.login.eq_ignore_ascii_case(author) || body != replacement.as_bytes() {
                return Err(unresolved());
            }
            "verified"
        }
    } else {
        "inspected"
    };
    let body_sha256 = replacement
        .as_ref()
        .map_or_else(|| digest(&body), |body| digest(body.as_bytes()));
    Ok(emit_success(
        schema_version_for(BINARY, OP, 1),
        Payload {
            provider: "github",
            repository: repo,
            id: args.id,
            author: &current.user.login,
            body_sha256,
            status,
            history_redacted: false,
        },
        format,
        |p| {
            println!(
                "comment.redact: {} id={} author={} body_sha256={}; edit history needs separate provider redaction",
                p.status, p.id, p.author, p.body_sha256
            )
        },
    ))
}
