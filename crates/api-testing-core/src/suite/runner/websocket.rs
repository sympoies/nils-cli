use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;
use crate::cli_util;
use crate::suite::resolve::{resolve_path_from_repo_root, write_file};
use crate::suite::runtime::{
    path_relative_to_repo_or_abs, plan_case_output_paths, resolve_ws_token_profile, resolve_ws_url,
};
use crate::suite::schema::{SuiteCase, SuiteDefaults};

pub(super) enum PrepareOutcome<T> {
    Ready(T),
}

pub(super) struct WebsocketCasePlan {
    pub(super) request_abs: PathBuf,
    pub(super) request_file: crate::websocket::schema::WebsocketRequestFile,
    pub(super) config_dir: String,
    pub(super) url: String,
    pub(super) token: String,
}

pub(super) struct WebsocketCaseRunOutput {
    pub(super) status: String,
    pub(super) message: Option<String>,
    pub(super) assertions: Option<serde_json::Value>,
    pub(super) command_snippet: Option<String>,
    pub(super) stdout_path: PathBuf,
    pub(super) stderr_path: PathBuf,
}

pub(super) fn prepare_websocket_case(
    repo_root: &Path,
    case: &SuiteCase,
    id: &str,
    defaults: &SuiteDefaults,
) -> Result<PrepareOutcome<WebsocketCasePlan>> {
    let request_rel = case.request.trim();
    if request_rel.is_empty() {
        anyhow::bail!("websocket case '{id}' is missing request");
    }
    let request_abs = resolve_path_from_repo_root(repo_root, request_rel);
    if !request_abs.is_file() {
        anyhow::bail!("websocket case '{id}' request not found: {request_rel}");
    }

    let config_dir = cli_util::trim_non_empty(&case.config_dir)
        .unwrap_or_else(|| defaults.websocket.config_dir.clone());
    let token =
        cli_util::trim_non_empty(&case.token).unwrap_or_else(|| defaults.websocket.token.clone());
    let url = cli_util::trim_non_empty(&case.url).unwrap_or_else(|| defaults.websocket.url.clone());

    let request_file = crate::websocket::schema::WebsocketRequestFile::load(&request_abs)?;

    Ok(PrepareOutcome::Ready(WebsocketCasePlan {
        request_abs,
        request_file,
        config_dir,
        url,
        token,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_websocket_case(
    repo_root: &Path,
    run_dir_abs: &Path,
    safe_id: &str,
    effective_no_history: bool,
    effective_env: &str,
    defaults: &SuiteDefaults,
    env_ws_url: &str,
    ws_config_dir: &str,
    ws_url: &str,
    ws_token: &str,
    request_abs: &Path,
    request_file: &crate::websocket::schema::WebsocketRequestFile,
) -> Result<WebsocketCaseRunOutput> {
    let outputs = plan_case_output_paths(run_dir_abs, safe_id);
    let stdout_path = outputs.stdout_path;
    let stderr_path = outputs.stderr_path;
    write_file(&stdout_path, b"")?;
    write_file(&stderr_path, b"")?;

    let mut status = "pending".to_string();
    let mut message: Option<String> = None;

    let target = match resolve_ws_url(
        repo_root,
        ws_config_dir,
        ws_url,
        effective_env,
        defaults,
        env_ws_url,
    ) {
        Ok(v) => v,
        Err(err) => {
            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
            status = "failed".to_string();
            message = Some("websocket_runner_failed".to_string());
            String::new()
        }
    };

    let mut assertions: Option<serde_json::Value> = None;

    if status != "failed" {
        let setup_dir_abs = resolve_path_from_repo_root(repo_root, ws_config_dir);
        let bearer = if !ws_token.trim().is_empty() {
            match resolve_ws_token_profile(&setup_dir_abs, ws_token) {
                Ok(t) => Some(t),
                Err(err) => {
                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                    status = "failed".to_string();
                    message = Some("websocket_runner_failed".to_string());
                    None
                }
            }
        } else {
            None
        };

        if status != "failed" {
            match crate::websocket::runner::execute_websocket_request(
                request_file,
                &target,
                bearer.as_deref(),
            ) {
                Ok(executed) => {
                    let transcript_json = serde_json::json!({
                        "target": executed.target,
                        "transcript": executed.transcript,
                        "lastReceived": executed.last_received,
                    });
                    let pretty = serde_json::to_vec_pretty(&transcript_json)
                        .context("serialize websocket transcript")?;
                    write_file(&stdout_path, &pretty)?;

                    let mut assert_rows: Vec<serde_json::Value> = Vec::new();
                    if let Some(expect) = request_file.request.expect.as_ref() {
                        if let Some(text_contains) = expect.text_contains.as_deref() {
                            let got = executed.last_received.as_deref().unwrap_or_default();
                            let state = if got.contains(text_contains) {
                                "passed"
                            } else {
                                "failed"
                            };
                            assert_rows.push(serde_json::json!({
                                "label": format!("expect.textContains: {text_contains}"),
                                "state": state,
                            }));
                        }
                        if let Some(jq_expr) = expect.jq.as_deref() {
                            let jq_state = if let Some(last) = executed.last_received.as_deref() {
                                match serde_json::from_str::<serde_json::Value>(last) {
                                    Ok(v) => {
                                        if crate::jq::eval_exit_status(&v, jq_expr).unwrap_or(false)
                                        {
                                            "passed"
                                        } else {
                                            "failed"
                                        }
                                    }
                                    Err(_) => "failed",
                                }
                            } else {
                                "failed"
                            };

                            assert_rows.push(serde_json::json!({
                                "label": format!("expect.jq: {jq_expr}"),
                                "state": jq_state,
                            }));
                        }
                    }
                    if !assert_rows.is_empty() {
                        assertions = Some(serde_json::json!({"checks": assert_rows}));
                    }

                    if let Err(err) = crate::websocket::expect::evaluate_main_response(
                        &request_file.request,
                        &executed,
                    ) {
                        write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                        status = "failed".to_string();
                        message = Some("websocket_runner_failed".to_string());
                    } else {
                        status = "passed".to_string();
                    }
                }
                Err(err) => {
                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                    status = "failed".to_string();
                    message = Some("websocket_runner_failed".to_string());
                }
            }
        }
    }

    let mut argv: Vec<String> = vec![
        "api-websocket".to_string(),
        "call".to_string(),
        "--config-dir".to_string(),
        ws_config_dir.to_string(),
    ];
    if effective_no_history {
        argv.push("--no-history".to_string());
    }
    if !ws_url.trim().is_empty() {
        argv.push("--url".to_string());
        argv.push(ws_url.to_string());
    } else if !effective_env.trim().is_empty() {
        argv.push("--env".to_string());
        argv.push(effective_env.to_string());
    }
    if !ws_token.trim().is_empty() {
        argv.push("--token".to_string());
        argv.push(ws_token.to_string());
    }
    argv.push(path_relative_to_repo_or_abs(repo_root, request_abs));
    let args = super::mask_args_for_command_snippet(&argv[1..]);
    let snippet = format!("{} {}", cli_util::shell_quote("api-websocket"), args);

    Ok(WebsocketCaseRunOutput {
        status,
        message,
        assertions,
        command_snippet: Some(snippet),
        stdout_path,
        stderr_path,
    })
}
