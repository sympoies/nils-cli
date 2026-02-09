use std::path::{Path, PathBuf};

use crate::Result;
use crate::cli_util;
use crate::suite::auth::SuiteAuthManager;
use crate::suite::resolve::{resolve_path_from_repo_root, write_file};
use crate::suite::runtime::{
    path_relative_to_repo_or_abs, plan_case_output_paths, resolve_rest_base_url,
    resolve_rest_token_profile,
};
use crate::suite::safety::{
    MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE, MSG_WRITE_CASES_DISABLED, rest_method_is_write,
    writes_enabled,
};
use crate::suite::schema::{SuiteCase, SuiteDefaults};

pub(super) enum PrepareOutcome<T> {
    Ready(T),
    Skipped { message: String },
    Failed { message: String },
}

pub(super) struct RestCasePlan {
    pub(super) request_abs: PathBuf,
    pub(super) request_file: crate::rest::schema::RestRequestFile,
    pub(super) config_dir: String,
    pub(super) url: String,
    pub(super) token: String,
    pub(super) access_token_for_case: String,
}

pub(super) struct RestFlowCasePlan {
    pub(super) login_abs: PathBuf,
    pub(super) request_abs: PathBuf,
    pub(super) login_request_file: crate::rest::schema::RestRequestFile,
    pub(super) main_request_file: crate::rest::schema::RestRequestFile,
    pub(super) config_dir: String,
    pub(super) url: String,
    pub(super) token_jq: String,
}

pub(super) struct RestCaseRunOutput {
    pub(super) status: String,
    pub(super) message: Option<String>,
    pub(super) command_snippet: Option<String>,
    pub(super) stdout_path: PathBuf,
    pub(super) stderr_path: PathBuf,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_rest_case(
    repo_root: &Path,
    case: &SuiteCase,
    id: &str,
    defaults: &SuiteDefaults,
    env_rest_url: &str,
    env_gql_url: &str,
    allow_writes_flag: bool,
    effective_env: &str,
    auth_manager: Option<&mut SuiteAuthManager>,
) -> Result<PrepareOutcome<RestCasePlan>> {
    let request_rel = case.request.trim();
    if request_rel.is_empty() {
        anyhow::bail!("REST case '{id}' is missing request");
    }
    let request_abs = resolve_path_from_repo_root(repo_root, request_rel);
    if !request_abs.is_file() {
        anyhow::bail!("REST case '{id}' request not found: {request_rel}");
    }

    let config_dir = cli_util::trim_non_empty(&case.config_dir)
        .unwrap_or_else(|| defaults.rest.config_dir.clone());
    let token =
        cli_util::trim_non_empty(&case.token).unwrap_or_else(|| defaults.rest.token.clone());

    let mut url = cli_util::trim_non_empty(&case.url).unwrap_or_else(|| defaults.rest.url.clone());
    if url.trim().is_empty() && !env_rest_url.trim().is_empty() {
        url = env_rest_url.to_string();
    }

    let request_file = crate::rest::schema::RestRequestFile::load(&request_abs)?;

    if rest_method_is_write(&request_file.request.method) {
        if !case.allow_write {
            return Ok(PrepareOutcome::Failed {
                message: MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE.to_string(),
            });
        }
        if !writes_enabled(allow_writes_flag, effective_env) {
            return Ok(PrepareOutcome::Skipped {
                message: MSG_WRITE_CASES_DISABLED.to_string(),
            });
        }
    }

    let mut access_token_for_case = String::new();
    if let Some(mgr) = auth_manager
        && !token.trim().is_empty()
    {
        match mgr.ensure_token(token.trim(), repo_root, defaults, env_rest_url, env_gql_url) {
            Ok(t) => access_token_for_case = t,
            Err(err) => {
                return Ok(PrepareOutcome::Failed { message: err });
            }
        }
    }

    Ok(PrepareOutcome::Ready(RestCasePlan {
        request_abs,
        request_file,
        config_dir,
        url,
        token,
        access_token_for_case,
    }))
}

pub(super) fn prepare_rest_flow_case(
    repo_root: &Path,
    case: &SuiteCase,
    id: &str,
    defaults: &SuiteDefaults,
    env_rest_url: &str,
    allow_writes_flag: bool,
    effective_env: &str,
) -> Result<PrepareOutcome<RestFlowCasePlan>> {
    let login_rel = case.login_request.trim();
    let request_rel = case.request.trim();
    if login_rel.is_empty() {
        anyhow::bail!("rest-flow case '{id}' is missing loginRequest");
    }
    if request_rel.is_empty() {
        anyhow::bail!("rest-flow case '{id}' is missing request");
    }
    let login_abs = resolve_path_from_repo_root(repo_root, login_rel);
    let request_abs = resolve_path_from_repo_root(repo_root, request_rel);
    if !login_abs.is_file() {
        anyhow::bail!("rest-flow case '{id}' loginRequest not found: {login_rel}");
    }
    if !request_abs.is_file() {
        anyhow::bail!("rest-flow case '{id}' request not found: {request_rel}");
    }

    let config_dir = cli_util::trim_non_empty(&case.config_dir)
        .unwrap_or_else(|| defaults.rest.config_dir.clone());
    let mut url = cli_util::trim_non_empty(&case.url).unwrap_or_else(|| defaults.rest.url.clone());
    if url.trim().is_empty() && !env_rest_url.trim().is_empty() {
        url = env_rest_url.to_string();
    }

    let token_jq = cli_util::trim_non_empty(&case.token_jq)
        .unwrap_or_else(super::context::default_rest_flow_token_jq);

    let login_request_file = crate::rest::schema::RestRequestFile::load(&login_abs)?;
    let main_request_file = crate::rest::schema::RestRequestFile::load(&request_abs)?;

    if rest_method_is_write(&login_request_file.request.method)
        || rest_method_is_write(&main_request_file.request.method)
    {
        if !case.allow_write {
            return Ok(PrepareOutcome::Failed {
                message: MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE.to_string(),
            });
        }
        if !writes_enabled(allow_writes_flag, effective_env) {
            return Ok(PrepareOutcome::Skipped {
                message: MSG_WRITE_CASES_DISABLED.to_string(),
            });
        }
    }

    Ok(PrepareOutcome::Ready(RestFlowCasePlan {
        login_abs,
        request_abs,
        login_request_file,
        main_request_file,
        config_dir,
        url,
        token_jq,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_rest_case(
    repo_root: &Path,
    run_dir_abs: &Path,
    safe_id: &str,
    effective_no_history: bool,
    effective_env: &str,
    defaults: &SuiteDefaults,
    env_rest_url: &str,
    rest_config_dir: &str,
    rest_url: &str,
    rest_token: &str,
    access_token_for_case: &str,
    request_abs: &Path,
    request_file: &crate::rest::schema::RestRequestFile,
) -> Result<RestCaseRunOutput> {
    let outputs = plan_case_output_paths(run_dir_abs, safe_id);
    let stdout_path = outputs.stdout_path;
    let stderr_path = outputs.stderr_path;
    write_file(&stdout_path, b"")?;
    write_file(&stderr_path, b"")?;

    let mut status = "pending".to_string();
    let mut message: Option<String> = None;

    let base_url = match resolve_rest_base_url(
        repo_root,
        rest_config_dir,
        rest_url,
        effective_env,
        defaults,
        env_rest_url,
    ) {
        Ok(v) => v,
        Err(err) => {
            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
            status = "failed".to_string();
            message = Some("rest_runner_failed".to_string());
            String::new()
        }
    };

    if status != "failed" {
        let setup_dir_abs = resolve_path_from_repo_root(repo_root, rest_config_dir);
        let bearer = if !access_token_for_case.trim().is_empty() {
            Some(access_token_for_case.trim().to_string())
        } else if !rest_token.trim().is_empty() {
            match resolve_rest_token_profile(&setup_dir_abs, rest_token) {
                Ok(t) => Some(t),
                Err(err) => {
                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                    status = "failed".to_string();
                    message = Some("rest_runner_failed".to_string());
                    None
                }
            }
        } else {
            None
        };

        if status != "failed" {
            match crate::rest::runner::execute_rest_request(
                request_file,
                &base_url,
                bearer.as_deref(),
            ) {
                Ok(executed) => {
                    write_file(&stdout_path, &executed.response.body)?;
                    if let Err(err) = crate::rest::expect::evaluate_main_response(
                        &request_file.request,
                        &executed,
                    ) {
                        write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                        status = "failed".to_string();
                        message = Some("rest_runner_failed".to_string());
                    } else {
                        status = "passed".to_string();
                    }
                }
                Err(err) => {
                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                    status = "failed".to_string();
                    message = Some("rest_runner_failed".to_string());
                }
            }
        }
    }

    let mut argv: Vec<String> = vec![
        "api-rest".to_string(),
        "call".to_string(),
        "--config-dir".to_string(),
        rest_config_dir.to_string(),
    ];
    if effective_no_history {
        argv.push("--no-history".to_string());
    }
    if !rest_url.trim().is_empty() {
        argv.push("--url".to_string());
        argv.push(rest_url.to_string());
    } else if !effective_env.trim().is_empty() {
        argv.push("--env".to_string());
        argv.push(effective_env.to_string());
    }
    if !rest_token.trim().is_empty() && access_token_for_case.trim().is_empty() {
        argv.push("--token".to_string());
        argv.push(rest_token.to_string());
    }
    argv.push(path_relative_to_repo_or_abs(repo_root, request_abs));

    let args = super::mask_args_for_command_snippet(&argv[1..]);
    let env_prefix = if !access_token_for_case.trim().is_empty() {
        "ACCESS_TOKEN=REDACTED REST_TOKEN_NAME= GQL_JWT_NAME="
    } else {
        ""
    };
    let snippet = if env_prefix.is_empty() {
        format!("{} {}", cli_util::shell_quote("api-rest"), args)
    } else {
        format!(
            "{env_prefix} {} {}",
            cli_util::shell_quote("api-rest"),
            args
        )
    };

    Ok(RestCaseRunOutput {
        status,
        message,
        command_snippet: Some(snippet.trim().to_string()),
        stdout_path,
        stderr_path,
    })
}
