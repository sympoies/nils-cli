use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;

use crate::cli_util;
use crate::suite::auth::{AuthInit, SuiteAuthManager};
use crate::suite::cleanup::{run_case_cleanup, CleanupContext};
use crate::suite::filter::selection_skip_reason;
use crate::suite::resolve::write_file;
use crate::suite::results::{SuiteCaseResult, SuiteRunResults, SuiteRunSummary};
use crate::suite::runtime::{
    path_relative_to_repo_or_abs, resolve_effective_env, resolve_effective_no_history,
    resolve_rest_base_url, sanitize_id, time_iso_now, time_run_id_now,
};
use crate::suite::schema::LoadedSuite;
use crate::Result;

mod context;
mod graphql;
mod progress;
mod rest;

pub use context::{SuiteRunOptions, SuiteRunOutput};

fn mask_args_for_command_snippet(args: &[String]) -> String {
    if args.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = Vec::new();
    let mut mask_next = false;
    for a in args {
        if mask_next {
            out.push("REDACTED".to_string());
            mask_next = false;
            continue;
        }
        if a == "--token" || a == "--jwt" {
            out.push(a.clone());
            mask_next = true;
            continue;
        }
        if let Some((k, _v)) = a.split_once('=') {
            if k == "--token" || k == "--jwt" {
                out.push(format!("{k}=REDACTED"));
                continue;
            }
        }
        out.push(a.clone());
    }

    out.into_iter()
        .map(|a| cli_util::shell_quote(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn run_suite(
    repo_root: &Path,
    loaded: LoadedSuite,
    mut options: SuiteRunOptions,
) -> Result<SuiteRunOutput> {
    let mut progress = progress::SuiteProgress::new(options.progress.take());

    let run_id = time_run_id_now()?;
    let started_at = time_iso_now()?;

    let run_dir_abs = options.output_dir_base.join(&run_id);
    std::fs::create_dir_all(&run_dir_abs)
        .with_context(|| format!("create output dir: {}", run_dir_abs.display()))?;

    let suite_file_rel = path_relative_to_repo_or_abs(repo_root, &loaded.suite_path);
    let output_dir_rel = path_relative_to_repo_or_abs(repo_root, &run_dir_abs);
    let suite_name_value = context::suite_display_name(&loaded);

    let defaults = &loaded.manifest.defaults;

    let mut auth_init_message: Option<String> = None;
    let mut auth_manager: Option<SuiteAuthManager> = match loaded.manifest.auth.clone() {
        None => None,
        Some(auth) => match SuiteAuthManager::init_from_suite(auth, defaults)? {
            AuthInit::Disabled { message } => {
                auth_init_message = message;
                None
            }
            AuthInit::Enabled(mgr) => Some(*mgr),
        },
    };

    let mut total: u32 = 0;
    let mut passed: u32 = 0;
    let mut failed: u32 = 0;
    let mut skipped: u32 = 0;

    let mut cases_out: Vec<SuiteCaseResult> = Vec::new();

    let mut case_index: u64 = 0;

    for c in &loaded.manifest.cases {
        total += 1;

        let id = c.id.trim().to_string();
        let safe_id = sanitize_id(&id);

        case_index = case_index.saturating_add(1);
        progress.on_case_start(case_index, if id.is_empty() { &safe_id } else { &id });

        let tags = c.tags.clone();
        let ty = context::case_type_normalized(&c.case_type);

        let effective_env = resolve_effective_env(&c.env, defaults);
        let effective_no_history = resolve_effective_no_history(c.no_history, defaults);

        if let Some(reason) = selection_skip_reason(
            &id,
            &tags,
            &options.required_tags,
            &options.only_ids,
            &options.skip_ids,
        ) {
            skipped += 1;
            cases_out.push(SuiteCaseResult {
                id,
                case_type: ty,
                status: "skipped".to_string(),
                duration_ms: 0,
                tags,
                command: None,
                message: Some(reason.as_str().to_string()),
                assertions: None,
                stdout_file: None,
                stderr_file: None,
            });
            continue;
        }

        let start = Instant::now();

        let mut status: String;
        let mut message: Option<String> = None;
        let mut assertions: Option<serde_json::Value> = None;
        let mut command_snippet: Option<String> = None;
        let mut stdout_file_abs: Option<PathBuf> = None;
        let mut stderr_file_abs: Option<PathBuf> = None;

        let mut rest_config_dir = String::new();
        let mut rest_url = String::new();
        let mut rest_token = String::new();
        let mut gql_config_dir = String::new();
        let mut gql_url = String::new();
        let mut gql_jwt = String::new();
        let mut access_token_for_case = String::new();

        match ty.as_str() {
            "rest" => {
                match rest::prepare_rest_case(
                    repo_root,
                    c,
                    &id,
                    defaults,
                    &options.env_rest_url,
                    &options.env_gql_url,
                    options.allow_writes_flag,
                    &effective_env,
                    auth_manager.as_mut(),
                )? {
                    rest::PrepareOutcome::Ready(plan) => {
                        let rest::RestCasePlan {
                            request_abs,
                            request_file,
                            config_dir,
                            url,
                            token,
                            access_token_for_case: prepared_access_token,
                        } = plan;

                        rest_config_dir = config_dir;
                        rest_url = url;
                        rest_token = token;
                        access_token_for_case = prepared_access_token;

                        let out = rest::run_rest_case(
                            repo_root,
                            &run_dir_abs,
                            &safe_id,
                            effective_no_history,
                            &effective_env,
                            defaults,
                            &options.env_rest_url,
                            &rest_config_dir,
                            &rest_url,
                            &rest_token,
                            &access_token_for_case,
                            &request_abs,
                            &request_file,
                        )?;

                        status = out.status;
                        message = out.message;
                        command_snippet = out.command_snippet;
                        stdout_file_abs = Some(out.stdout_path);
                        stderr_file_abs = Some(out.stderr_path);

                        match status.as_str() {
                            "passed" => passed += 1,
                            "failed" => failed += 1,
                            _ => {}
                        }
                    }
                    rest::PrepareOutcome::Skipped { message: msg } => {
                        status = "skipped".to_string();
                        message = Some(msg);
                        skipped += 1;
                    }
                    rest::PrepareOutcome::Failed { message: msg } => {
                        status = "failed".to_string();
                        message = Some(msg);
                        failed += 1;
                    }
                }
            }
            "rest-flow" | "rest_flow" => {
                match rest::prepare_rest_flow_case(
                    repo_root,
                    c,
                    &id,
                    defaults,
                    &options.env_rest_url,
                    options.allow_writes_flag,
                    &effective_env,
                )? {
                    rest::PrepareOutcome::Ready(plan) => {
                        let login_abs = plan.login_abs;
                        let request_abs = plan.request_abs;
                        let login_request_file = plan.login_request_file;
                        let main_request_file = plan.main_request_file;
                        rest_config_dir = plan.config_dir;
                        rest_url = plan.url;
                        let token_jq = plan.token_jq;

                        let stderr_path = run_dir_abs.join(format!("{safe_id}.stderr.log"));
                        write_file(&stderr_path, b"")?;
                        stderr_file_abs = Some(stderr_path.clone());

                        let base_url = match resolve_rest_base_url(
                            repo_root,
                            &rest_config_dir,
                            &rest_url,
                            &effective_env,
                            defaults,
                            &options.env_rest_url,
                        ) {
                            Ok(v) => v,
                            Err(err) => {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("rest_flow_login_failed".to_string());
                                failed += 1;
                                cases_out.push(case_result(
                                    &id,
                                    &ty,
                                    &tags,
                                    &status,
                                    start.elapsed(),
                                    command_snippet.clone(),
                                    message.clone(),
                                    assertions.clone(),
                                    None,
                                    Some(&stderr_path),
                                    repo_root,
                                ));
                                if options.fail_fast && status == "failed" {
                                    break;
                                }
                                continue;
                            }
                        };

                        // Command snippet (parity intent; uses jq extraction).
                        let mut login_args: Vec<String> = Vec::new();
                        login_args.push("call".to_string());
                        login_args.push("--config-dir".to_string());
                        login_args.push(rest_config_dir.clone());
                        if effective_no_history {
                            login_args.push("--no-history".to_string());
                        }
                        if !rest_url.trim().is_empty() {
                            login_args.push("--url".to_string());
                            login_args.push(rest_url.clone());
                        } else if !effective_env.trim().is_empty() {
                            login_args.push("--env".to_string());
                            login_args.push(effective_env.clone());
                        }
                        login_args.push(path_relative_to_repo_or_abs(repo_root, &login_abs));

                        let mut main_args: Vec<String> = Vec::new();
                        main_args.push("call".to_string());
                        main_args.push("--config-dir".to_string());
                        main_args.push(rest_config_dir.clone());
                        if effective_no_history {
                            main_args.push("--no-history".to_string());
                        }
                        if !rest_url.trim().is_empty() {
                            main_args.push("--url".to_string());
                            main_args.push(rest_url.clone());
                        } else if !effective_env.trim().is_empty() {
                            main_args.push("--env".to_string());
                            main_args.push(effective_env.clone());
                        }
                        main_args.push(path_relative_to_repo_or_abs(repo_root, &request_abs));

                        let login_args_snip = mask_args_for_command_snippet(&login_args);
                        let main_args_snip = mask_args_for_command_snippet(&main_args);
                        let token_expr_q = cli_util::shell_quote(&token_jq);
                        command_snippet = Some(format!(
                        "ACCESS_TOKEN=\"$(REST_TOKEN_NAME= ACCESS_TOKEN= {} {} | jq -r {token_expr_q})\" REST_TOKEN_NAME= {} {}",
                        cli_util::shell_quote("api-rest"),
                        login_args_snip,
                        cli_util::shell_quote("api-rest"),
                        main_args_snip
                    ));

                        // Login request
                        let login_executed = match crate::rest::runner::execute_rest_request(
                            &login_request_file,
                            &base_url,
                            None,
                        ) {
                            Ok(v) => v,
                            Err(err) => {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("rest_flow_login_failed".to_string());
                                failed += 1;
                                cases_out.push(case_result(
                                    &id,
                                    &ty,
                                    &tags,
                                    &status,
                                    start.elapsed(),
                                    command_snippet,
                                    message,
                                    assertions,
                                    None,
                                    Some(&stderr_path),
                                    repo_root,
                                ));
                                if options.fail_fast && status == "failed" {
                                    break;
                                }
                                continue;
                            }
                        };

                        if let Err(err) = crate::rest::expect::evaluate_main_response(
                            &login_request_file.request,
                            &login_executed,
                        ) {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("rest_flow_login_failed".to_string());
                            failed += 1;
                            cases_out.push(case_result(
                                &id,
                                &ty,
                                &tags,
                                &status,
                                start.elapsed(),
                                command_snippet,
                                message,
                                assertions,
                                None,
                                Some(&stderr_path),
                                repo_root,
                            ));
                            if options.fail_fast && status == "failed" {
                                break;
                            }
                            continue;
                        }

                        let login_json: serde_json::Value =
                            match serde_json::from_slice(&login_executed.response.body) {
                                Ok(v) => v,
                                Err(_) => serde_json::Value::Null,
                            };
                        let token = crate::jq::query_raw(&login_json, &token_jq)
                            .ok()
                            .and_then(|lines| lines.into_iter().next())
                            .unwrap_or_default();
                        let token = token.trim().to_string();
                        if token.is_empty() || token == "null" {
                            let hint = "Failed to extract token from login response.\nHint: set cases[i].tokenJq to the token field (e.g. .accessToken).\n";
                            write_file(&stderr_path, hint.as_bytes())?;
                            status = "failed".to_string();
                            message = Some("rest_flow_token_extract_failed".to_string());
                            failed += 1;
                            cases_out.push(case_result(
                                &id,
                                &ty,
                                &tags,
                                &status,
                                start.elapsed(),
                                command_snippet,
                                message,
                                assertions,
                                None,
                                Some(&stderr_path),
                                repo_root,
                            ));
                            if options.fail_fast && status == "failed" {
                                break;
                            }
                            continue;
                        }

                        // Main request with extracted token
                        let stdout_path = run_dir_abs.join(format!("{safe_id}.response.json"));
                        write_file(&stdout_path, b"")?;
                        stdout_file_abs = Some(stdout_path.clone());

                        match crate::rest::runner::execute_rest_request(
                            &main_request_file,
                            &base_url,
                            Some(&token),
                        ) {
                            Ok(executed) => {
                                write_file(&stdout_path, &executed.response.body)?;
                                if let Err(err) = crate::rest::expect::evaluate_main_response(
                                    &main_request_file.request,
                                    &executed,
                                ) {
                                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                    status = "failed".to_string();
                                    message = Some("rest_flow_request_failed".to_string());
                                    failed += 1;
                                } else {
                                    status = "passed".to_string();
                                    passed += 1;
                                }
                            }
                            Err(err) => {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("rest_flow_request_failed".to_string());
                                failed += 1;
                            }
                        }
                    }
                    rest::PrepareOutcome::Skipped { message: msg } => {
                        status = "skipped".to_string();
                        message = Some(msg);
                        skipped += 1;
                    }
                    rest::PrepareOutcome::Failed { message: msg } => {
                        status = "failed".to_string();
                        message = Some(msg);
                        failed += 1;
                    }
                }
            }
            "graphql" => {
                match graphql::prepare_graphql_case(
                    repo_root,
                    c,
                    &id,
                    defaults,
                    &options.env_rest_url,
                    &options.env_gql_url,
                    options.allow_writes_flag,
                    &effective_env,
                    auth_manager.as_mut(),
                )? {
                    graphql::PrepareOutcome::Ready(plan) => {
                        let graphql::GraphqlCasePlan {
                            op_abs,
                            vars_abs,
                            config_dir,
                            url,
                            jwt,
                            access_token_for_case: prepared_access_token,
                        } = plan;

                        gql_config_dir = config_dir;
                        gql_url = url;
                        gql_jwt = jwt;
                        access_token_for_case = prepared_access_token;

                        let expect_jq_raw = c.expect.as_ref().map(|e| e.jq.as_str()).unwrap_or("");
                        let graphql::GraphqlCaseRunOutput {
                            status: graphql_status,
                            message: graphql_message,
                            assertions: graphql_assertions,
                            command_snippet: graphql_command_snippet,
                            stdout_path,
                            stderr_path,
                            skip_cleanup,
                        } = graphql::run_graphql_case(
                            repo_root,
                            &run_dir_abs,
                            &safe_id,
                            effective_no_history,
                            &effective_env,
                            defaults,
                            &options.env_gql_url,
                            &gql_config_dir,
                            &gql_url,
                            &gql_jwt,
                            &access_token_for_case,
                            &op_abs,
                            vars_abs.as_deref(),
                            c.allow_errors,
                            expect_jq_raw,
                        )?;

                        status = graphql_status;
                        message = graphql_message;
                        assertions = graphql_assertions;
                        command_snippet = graphql_command_snippet;
                        stdout_file_abs = Some(stdout_path);
                        stderr_file_abs = Some(stderr_path);

                        match status.as_str() {
                            "passed" => passed += 1,
                            "failed" => failed += 1,
                            _ => {}
                        }

                        if skip_cleanup {
                            cases_out.push(case_result(
                                &id,
                                &ty,
                                &tags,
                                &status,
                                start.elapsed(),
                                command_snippet.clone(),
                                message.clone(),
                                assertions.clone(),
                                stdout_file_abs.as_deref(),
                                stderr_file_abs.as_deref(),
                                repo_root,
                            ));

                            if options.fail_fast && status == "failed" {
                                break;
                            }
                            continue;
                        }
                    }
                    graphql::PrepareOutcome::Skipped { message: msg } => {
                        status = "skipped".to_string();
                        message = Some(msg);
                        skipped += 1;
                    }
                    graphql::PrepareOutcome::Failed { message: msg } => {
                        status = "failed".to_string();
                        message = Some(msg);
                        failed += 1;
                    }
                }
            }
            other => {
                anyhow::bail!("Unknown case type '{other}' for case '{id}'");
            }
        }

        let duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

        // Cleanup hook (best-effort; can flip passed->failed).
        if status == "passed" || status == "failed" {
            let cleanup_ok = if let (Some(stderr_path), Some(cleanup)) =
                (stderr_file_abs.as_deref(), c.cleanup.as_ref())
            {
                let mut ctx = CleanupContext {
                    repo_root,
                    run_dir: &run_dir_abs,
                    case_id: &id,
                    safe_id: &safe_id,
                    main_response_file: stdout_file_abs.as_deref(),
                    main_stderr_file: stderr_path,
                    allow_writes_flag: options.allow_writes_flag,
                    effective_env: &effective_env,
                    effective_no_history,
                    suite_defaults: defaults,
                    env_rest_url: &options.env_rest_url,
                    env_gql_url: &options.env_gql_url,
                    rest_config_dir: &rest_config_dir,
                    rest_url: &rest_url,
                    rest_token: &rest_token,
                    gql_config_dir: &gql_config_dir,
                    gql_url: &gql_url,
                    gql_jwt: &gql_jwt,
                    access_token_for_case: &access_token_for_case,
                    auth_manager: auth_manager.as_mut(),
                    cleanup: Some(cleanup),
                };
                run_case_cleanup(&mut ctx)?
            } else {
                true
            };

            if !cleanup_ok && status == "passed" {
                status = "failed".to_string();
                message = Some("cleanup_failed".to_string());
                passed = passed.saturating_sub(1);
                failed += 1;
            }
        }

        cases_out.push(SuiteCaseResult {
            id,
            case_type: ty,
            status: status.clone(),
            duration_ms,
            tags,
            command: command_snippet,
            message,
            assertions,
            stdout_file: stdout_file_abs
                .as_deref()
                .map(|p| path_relative_to_repo_or_abs(repo_root, p)),
            stderr_file: stderr_file_abs
                .as_deref()
                .map(|p| path_relative_to_repo_or_abs(repo_root, p)),
        });

        if options.fail_fast && status == "failed" {
            break;
        }
    }

    let finished_at = time_iso_now()?;

    let results = SuiteRunResults {
        version: 1,
        suite: suite_name_value,
        suite_file: suite_file_rel,
        run_id,
        started_at,
        finished_at,
        output_dir: output_dir_rel,
        summary: SuiteRunSummary {
            total,
            passed,
            failed,
            skipped,
        },
        cases: cases_out,
    };

    if let Some(msg) = auth_init_message {
        // Best-effort: write to runner stderr file in output dir, to avoid losing context.
        let path = run_dir_abs.join("auth.disabled.log");
        let _ = write_file(&path, format!("{msg}\n").as_bytes());
    }

    Ok(SuiteRunOutput {
        run_dir_abs,
        results,
    })
}

#[allow(clippy::too_many_arguments)]
fn case_result(
    id: &str,
    case_type: &str,
    tags: &[String],
    status: &str,
    duration: std::time::Duration,
    command: Option<String>,
    message: Option<String>,
    assertions: Option<serde_json::Value>,
    stdout_file: Option<&Path>,
    stderr_file: Option<&Path>,
    repo_root: &Path,
) -> SuiteCaseResult {
    SuiteCaseResult {
        id: id.to_string(),
        case_type: case_type.to_string(),
        status: status.to_string(),
        duration_ms: duration.as_millis().try_into().unwrap_or(u64::MAX),
        tags: tags.to_vec(),
        command,
        message,
        assertions,
        stdout_file: stdout_file.map(|p| path_relative_to_repo_or_abs(repo_root, p)),
        stderr_file: stderr_file.map(|p| path_relative_to_repo_or_abs(repo_root, p)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::runtime;
    use crate::suite::safety::MSG_WRITE_CASES_DISABLED;
    use crate::suite::schema::{SuiteCase, SuiteDefaults, SuiteManifest};
    use nils_term::progress::Progress;
    use nils_term::progress::{ProgressDrawTarget, ProgressEnabled, ProgressOptions};
    use nils_test_support::fixtures::{GraphqlSetupFixture, RestSetupFixture};
    use pretty_assertions::assert_eq;
    use std::collections::HashSet;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use tempfile::TempDir;

    fn base_case(id: &str, case_type: &str) -> SuiteCase {
        SuiteCase {
            id: id.to_string(),
            case_type: case_type.to_string(),
            tags: Vec::new(),
            env: String::new(),
            no_history: None,
            allow_write: false,
            config_dir: String::new(),
            url: String::new(),
            token: String::new(),
            request: String::new(),
            login_request: String::new(),
            token_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars: None,
            allow_errors: false,
            expect: None,
            cleanup: None,
        }
    }

    struct TestServer {
        base_url: String,
        shutdown: mpsc::Sender<()>,
        join: Option<thread::JoinHandle<()>>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            let _ = self.shutdown.send(());
            if let Some(j) = self.join.take() {
                let _ = j.join();
            }
        }
    }

    fn read_until_headers_end(stream: &mut TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    if buf.len() > 64 * 1024 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn write_json_response(stream: &mut TcpStream, body: &[u8]) {
        let mut resp = Vec::new();
        resp.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
        resp.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
        resp.extend_from_slice(b"Content-Type: application/json\r\n");
        resp.extend_from_slice(b"\r\n");
        resp.extend_from_slice(body);
        let _ = stream.write_all(&resp);
        let _ = stream.flush();
    }

    fn start_server() -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let base_url = format!("http://{addr}");

        let (tx, rx) = mpsc::channel::<()>();
        let join = thread::spawn(move || loop {
            if rx.try_recv().is_ok() {
                break;
            }
            match listener.accept() {
                Ok((mut stream, _peer)) => {
                    read_until_headers_end(&mut stream);
                    write_json_response(&mut stream, br#"{"ok":true}"#);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        });

        TestServer {
            base_url,
            shutdown: tx,
            join: Some(join),
        }
    }

    fn read_output(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8_lossy(&buffer.lock().expect("buffer lock")).to_string()
    }

    fn normalize_progress_output(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' {
                i += 1;
                continue;
            }

            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if b.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }

            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    #[test]
    fn suite_runner_sanitize_id_matches_expected_replacements() {
        assert_eq!(runtime::sanitize_id("rest.health"), "rest.health");
        assert_eq!(runtime::sanitize_id("a b c"), "a-b-c");
        assert_eq!(runtime::sanitize_id(""), "case");
    }

    #[test]
    fn suite_runner_sanitize_id_handles_punctuation_and_unicode() {
        assert_eq!(runtime::sanitize_id("foo,bar"), "foo-bar");
        assert_eq!(runtime::sanitize_id("foo✅bar"), "foo-bar");
        assert_eq!(runtime::sanitize_id("✅foo"), "foo");
        assert_eq!(runtime::sanitize_id("foo!!!"), "foo");
        assert_eq!(runtime::sanitize_id("!!!"), "case");
    }

    #[test]
    fn suite_runner_masks_token_args_in_command_snippet() {
        let args = vec![
            "--token".to_string(),
            "secret".to_string(),
            "--jwt=jwt-value".to_string(),
            "--other".to_string(),
            "keep".to_string(),
            "--token=inline".to_string(),
            "--jwt".to_string(),
            "another".to_string(),
        ];

        let masked = mask_args_for_command_snippet(&args);

        assert_eq!(
            masked,
            "'--token' 'REDACTED' '--jwt=REDACTED' '--other' 'keep' '--token=REDACTED' '--jwt' 'REDACTED'"
        );
    }

    #[test]
    fn suite_runner_resolve_rest_base_url_precedence() {
        let fixture = RestSetupFixture::new();
        fixture.write_endpoints_env("REST_URL_STAGE=http://env-file\n");

        let defaults = SuiteDefaults::default();
        assert_eq!(
            runtime::resolve_rest_base_url(&fixture.root, "setup/rest", "", "", &defaults, "")
                .unwrap(),
            "http://localhost:6700"
        );
        assert_eq!(
            runtime::resolve_rest_base_url(&fixture.root, "setup/rest", "", "stage", &defaults, "")
                .unwrap(),
            "http://env-file"
        );
        assert_eq!(
            runtime::resolve_rest_base_url(
                &fixture.root,
                "setup/rest",
                "",
                "stage",
                &defaults,
                "http://env-var"
            )
            .unwrap(),
            "http://env-var"
        );

        let mut defaults_with_url = SuiteDefaults::default();
        defaults_with_url.rest.url = "http://default".to_string();
        assert_eq!(
            runtime::resolve_rest_base_url(
                &fixture.root,
                "setup/rest",
                "",
                "stage",
                &defaults_with_url,
                "http://env-var"
            )
            .unwrap(),
            "http://default"
        );
        assert_eq!(
            runtime::resolve_rest_base_url(
                &fixture.root,
                "setup/rest",
                "http://override",
                "stage",
                &defaults_with_url,
                "http://env-var"
            )
            .unwrap(),
            "http://override"
        );
    }

    #[test]
    fn suite_runner_resolve_gql_url_precedence() {
        let fixture = GraphqlSetupFixture::new();
        fixture.write_endpoints_env("GQL_URL_STAGE=http://env-file/graphql\n");

        let defaults = SuiteDefaults::default();
        assert_eq!(
            runtime::resolve_gql_url(&fixture.root, "setup/graphql", "", "", &defaults, "")
                .unwrap(),
            "http://localhost:6700/graphql"
        );
        assert_eq!(
            runtime::resolve_gql_url(&fixture.root, "setup/graphql", "", "stage", &defaults, "")
                .unwrap(),
            "http://env-file/graphql"
        );
        assert_eq!(
            runtime::resolve_gql_url(
                &fixture.root,
                "setup/graphql",
                "",
                "stage",
                &defaults,
                "http://env-var/graphql"
            )
            .unwrap(),
            "http://env-var/graphql"
        );

        let mut defaults_with_url = SuiteDefaults::default();
        defaults_with_url.graphql.url = "http://default/graphql".to_string();
        assert_eq!(
            runtime::resolve_gql_url(
                &fixture.root,
                "setup/graphql",
                "",
                "stage",
                &defaults_with_url,
                "http://env-var/graphql"
            )
            .unwrap(),
            "http://default/graphql"
        );
        assert_eq!(
            runtime::resolve_gql_url(
                &fixture.root,
                "setup/graphql",
                "http://override/graphql",
                "stage",
                &defaults_with_url,
                "http://env-var/graphql"
            )
            .unwrap(),
            "http://override/graphql"
        );
    }

    #[test]
    fn suite_runner_resolve_rest_token_profile_from_tokens_files() {
        let fixture = RestSetupFixture::new();
        fixture.write_tokens_env("REST_TOKEN_TEAM_ALPHA=base\n");
        fixture.write_tokens_local_env("REST_TOKEN_TEAM_ALPHA=local\n");

        let token = runtime::resolve_rest_token_profile(&fixture.setup_dir, "team alpha").unwrap();
        assert_eq!(token, "local");
    }

    #[test]
    fn suite_runner_token_profile_ignores_leading_separators() {
        let fixture = RestSetupFixture::new();
        fixture.write_tokens_env("REST_TOKEN_TEAM_ALPHA=base\n");

        let token = runtime::resolve_rest_token_profile(&fixture.setup_dir, "-team alpha").unwrap();
        assert_eq!(token, "base");
    }

    #[test]
    fn suite_runner_empty_suite_has_zero_counts() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults: SuiteDefaults::default(),
            auth: None,
            cases: Vec::new(),
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let options = SuiteRunOptions {
            required_tags: Vec::new(),
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: true,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            progress: None,
        };

        let out = run_suite(tmp.path(), loaded, options).unwrap();
        assert_eq!(out.results.summary.total, 0);
        assert_eq!(out.results.summary.passed, 0);
        assert_eq!(out.results.summary.failed, 0);
        assert_eq!(out.results.summary.skipped, 0);
        assert!(out.run_dir_abs.is_dir());
    }

    #[test]
    fn suite_runner_skips_case_when_tag_mismatch() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let mut case = base_case("case-1", "rest");
        case.tags = vec!["smoke".to_string()];

        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults: SuiteDefaults::default(),
            auth: None,
            cases: vec![case],
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let options = SuiteRunOptions {
            required_tags: vec!["fast".to_string()],
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: true,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            progress: None,
        };

        let out = run_suite(tmp.path(), loaded, options).unwrap();
        assert_eq!(out.results.summary.total, 1);
        assert_eq!(out.results.summary.skipped, 1);
        assert_eq!(out.results.cases[0].status, "skipped");
        assert_eq!(
            out.results.cases[0].message.as_deref(),
            Some("tag_mismatch")
        );
    }

    #[test]
    fn suite_runner_skips_write_case_when_writes_disabled() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let request_path = tmp.path().join("requests/post.request.json");
        std::fs::create_dir_all(request_path.parent().unwrap()).unwrap();
        std::fs::write(
            &request_path,
            br#"{"method":"POST","path":"/do","expect":{"status":200}}"#,
        )
        .unwrap();

        let mut case = base_case("write-case", "rest");
        case.allow_write = true;
        case.request = "requests/post.request.json".to_string();

        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults: SuiteDefaults::default(),
            auth: None,
            cases: vec![case],
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let options = SuiteRunOptions {
            required_tags: Vec::new(),
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: false,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            progress: None,
        };

        let out = run_suite(tmp.path(), loaded, options).unwrap();
        assert_eq!(out.results.summary.total, 1);
        assert_eq!(out.results.summary.skipped, 1);
        assert_eq!(out.results.cases[0].status, "skipped");
        assert_eq!(
            out.results.cases[0].message.as_deref(),
            Some(MSG_WRITE_CASES_DISABLED)
        );
    }

    #[test]
    fn suite_runner_unknown_case_type_is_error() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let case = base_case("case-1", "weird");
        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults: SuiteDefaults::default(),
            auth: None,
            cases: vec![case],
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let options = SuiteRunOptions {
            required_tags: Vec::new(),
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: true,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            progress: None,
        };

        let err = run_suite(tmp.path(), loaded, options).unwrap_err();
        assert!(format!("{err:#}").contains("Unknown case type 'weird'"));
    }

    #[test]
    fn suite_runner_no_history_flag_in_command_snippet() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let request_path = tmp.path().join("requests/health.request.json");
        std::fs::create_dir_all(request_path.parent().unwrap()).unwrap();
        std::fs::write(
            &request_path,
            br#"{"method":"GET","path":"/health","expect":{"status":200}}"#,
        )
        .unwrap();

        let server = start_server();

        let defaults = SuiteDefaults {
            no_history: true,
            ..Default::default()
        };

        let mut case = base_case("case-1", "rest");
        case.request = "requests/health.request.json".to_string();

        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults,
            auth: None,
            cases: vec![case],
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let options = SuiteRunOptions {
            required_tags: Vec::new(),
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: true,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: server.base_url.clone(),
            env_gql_url: String::new(),
            progress: None,
        };

        let out = run_suite(tmp.path(), loaded, options).unwrap();
        let cmd = out.results.cases[0].command.as_deref().unwrap_or("");
        assert!(cmd.contains("--no-history"));
        assert!(cmd.contains("--url"));
    }

    #[test]
    fn suite_runner_progress_disabled_is_noop() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let mut case = base_case("case-1", "rest");
        case.tags = vec!["smoke".to_string()];

        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults: SuiteDefaults::default(),
            auth: None,
            cases: vec![case],
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let progress = Progress::new(
            1,
            ProgressOptions::default()
                .with_enabled(ProgressEnabled::Off)
                .with_draw_target(ProgressDrawTarget::to_writer(buffer.clone()))
                .with_width(Some(60)),
        );

        let options = SuiteRunOptions {
            required_tags: vec!["fast".to_string()],
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: true,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            progress: Some(progress),
        };

        let out = run_suite(tmp.path(), loaded, options).unwrap();
        assert_eq!(out.results.summary.total, 1);
        assert_eq!(out.results.summary.skipped, 1);
        assert!(read_output(&buffer).is_empty());
    }

    #[test]
    fn suite_runner_progress_updates_position_and_message() {
        let tmp = TempDir::new().unwrap();
        let suite_path = tmp.path().join("suite.json");
        std::fs::write(&suite_path, br#"{"version":1,"cases":[]}"#).unwrap();

        let mut case = base_case("case-1", "rest");
        case.tags = vec!["smoke".to_string()];

        let manifest = SuiteManifest {
            version: 1,
            name: String::new(),
            defaults: SuiteDefaults::default(),
            auth: None,
            cases: vec![case],
        };
        let loaded = LoadedSuite {
            suite_path: suite_path.clone(),
            manifest,
        };

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let progress = Progress::new(
            1,
            ProgressOptions::default()
                .with_enabled(ProgressEnabled::Auto)
                .with_draw_target(ProgressDrawTarget::to_writer(buffer.clone()))
                .with_width(Some(60)),
        );

        let options = SuiteRunOptions {
            required_tags: vec!["fast".to_string()],
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: true,
            fail_fast: false,
            output_dir_base: tmp.path().join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            progress: Some(progress),
        };

        let out = run_suite(tmp.path(), loaded, options).unwrap();
        assert_eq!(out.results.summary.total, 1);
        assert_eq!(out.results.summary.skipped, 1);

        let normalized = normalize_progress_output(&read_output(&buffer));
        assert!(normalized.contains("1/1"), "output was: {normalized:?}");
        assert!(normalized.contains("case-1"), "output was: {normalized:?}");
    }
}
