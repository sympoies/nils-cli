use std::path::{Path, PathBuf};

use crate::Result;
use crate::cli_util;
use crate::suite::auth::SuiteAuthManager;
use crate::suite::resolve::{resolve_path_from_repo_root, write_file};
use crate::suite::runtime::{
    path_relative_to_repo_or_abs, plan_case_output_paths, resolve_gql_url,
    resolve_graphql_bearer_token,
};
use crate::suite::safety::graphql_safety_decision;
use crate::suite::schema::{SuiteCase, SuiteDefaults};

pub(super) enum PrepareOutcome<T> {
    Ready(T),
    Skipped { message: String },
    Failed { message: String },
}

pub(super) struct GraphqlCasePlan {
    pub(super) op_abs: PathBuf,
    pub(super) vars_abs: Option<PathBuf>,
    pub(super) config_dir: String,
    pub(super) url: String,
    pub(super) jwt: String,
    pub(super) access_token_for_case: String,
}

pub(super) struct GraphqlCaseRunOutput {
    pub(super) status: String,
    pub(super) message: Option<String>,
    pub(super) assertions: Option<serde_json::Value>,
    pub(super) command_snippet: Option<String>,
    pub(super) stdout_path: PathBuf,
    pub(super) stderr_path: PathBuf,
    pub(super) skip_cleanup: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_graphql_case(
    repo_root: &Path,
    case: &SuiteCase,
    id: &str,
    defaults: &SuiteDefaults,
    env_rest_url: &str,
    env_gql_url: &str,
    allow_writes_flag: bool,
    effective_env: &str,
    auth_manager: Option<&mut SuiteAuthManager>,
) -> Result<PrepareOutcome<GraphqlCasePlan>> {
    let op_rel = case.op.trim();
    if op_rel.is_empty() {
        anyhow::bail!("GraphQL case '{id}' is missing op");
    }
    let op_abs = resolve_path_from_repo_root(repo_root, op_rel);
    if !op_abs.is_file() {
        anyhow::bail!("GraphQL case '{id}' op not found: {op_rel}");
    }

    let vars_abs = case
        .vars
        .as_deref()
        .and_then(cli_util::trim_non_empty)
        .map(|p| resolve_path_from_repo_root(repo_root, &p));
    if let Some(vp) = vars_abs.as_deref()
        && !vp.is_file()
    {
        anyhow::bail!("GraphQL case '{id}' vars not found: {}", vp.display());
    }

    let config_dir = cli_util::trim_non_empty(&case.config_dir)
        .unwrap_or_else(|| defaults.graphql.config_dir.clone());
    let jwt = cli_util::trim_non_empty(&case.jwt).unwrap_or_else(|| defaults.graphql.jwt.clone());

    let mut url =
        cli_util::trim_non_empty(&case.url).unwrap_or_else(|| defaults.graphql.url.clone());
    if url.trim().is_empty() && !env_gql_url.trim().is_empty() {
        url = env_gql_url.to_string();
    }

    match graphql_safety_decision(&op_abs, case.allow_write, allow_writes_flag, effective_env)? {
        crate::suite::safety::SafetyDecision::Fail(msg) => {
            return Ok(PrepareOutcome::Failed {
                message: msg.to_string(),
            });
        }
        crate::suite::safety::SafetyDecision::Skip(msg) => {
            return Ok(PrepareOutcome::Skipped {
                message: msg.to_string(),
            });
        }
        crate::suite::safety::SafetyDecision::Allow => {}
    }

    let mut access_token_for_case = String::new();
    if let Some(mgr) = auth_manager
        && !jwt.trim().is_empty()
    {
        match mgr.ensure_token(jwt.trim(), repo_root, defaults, env_rest_url, env_gql_url) {
            Ok(t) => access_token_for_case = t,
            Err(err) => return Ok(PrepareOutcome::Failed { message: err }),
        }
    }

    Ok(PrepareOutcome::Ready(GraphqlCasePlan {
        op_abs,
        vars_abs,
        config_dir,
        url,
        jwt,
        access_token_for_case,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_graphql_case(
    repo_root: &Path,
    run_dir_abs: &Path,
    safe_id: &str,
    effective_no_history: bool,
    effective_env: &str,
    defaults: &SuiteDefaults,
    env_gql_url: &str,
    gql_config_dir: &str,
    gql_url: &str,
    gql_jwt: &str,
    access_token_for_case: &str,
    op_abs: &Path,
    vars_abs: Option<&Path>,
    allow_errors: bool,
    expect_jq_raw: &str,
) -> Result<GraphqlCaseRunOutput> {
    let outputs = plan_case_output_paths(run_dir_abs, safe_id);
    let stdout_path = outputs.stdout_path;
    let stderr_path = outputs.stderr_path;
    write_file(&stdout_path, b"")?;
    write_file(&stderr_path, b"")?;

    let mut status = "pending".to_string();
    let mut message: Option<String> = None;
    let mut assertions: Option<serde_json::Value> = None;

    let endpoint_url = match resolve_gql_url(
        repo_root,
        gql_config_dir,
        gql_url,
        effective_env,
        defaults,
        env_gql_url,
    ) {
        Ok(v) => v,
        Err(err) => {
            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
            status = "failed".to_string();
            message = Some("graphql_runner_failed".to_string());
            String::new()
        }
    };

    let vars_min_limit =
        cli_util::parse_u64_default(std::env::var("GQL_VARS_MIN_LIMIT").ok(), 5, 0);
    let vars_json = match vars_abs {
        None => None,
        Some(path) => {
            match crate::graphql::vars::GraphqlVariablesFile::load(path, vars_min_limit) {
                Ok(v) => Some(v.variables),
                Err(err) => {
                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                    status = "failed".to_string();
                    message = Some("graphql_runner_failed".to_string());
                    None
                }
            }
        }
    };

    let mut auth_stderr: Vec<u8> = Vec::new();
    let setup_dir_abs = resolve_path_from_repo_root(repo_root, gql_config_dir);
    let bearer = if status == "failed" {
        None
    } else if !access_token_for_case.trim().is_empty() {
        Some(access_token_for_case.trim().to_string())
    } else if !gql_jwt.trim().is_empty() {
        match resolve_graphql_bearer_token(
            &setup_dir_abs,
            &endpoint_url,
            op_abs,
            gql_jwt,
            &mut auth_stderr,
        ) {
            Ok(v) => v,
            Err(err) => {
                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                status = "failed".to_string();
                message = Some("graphql_runner_failed".to_string());
                None
            }
        }
    } else {
        None
    };

    if !auth_stderr.is_empty() {
        write_file(&stderr_path, &auth_stderr)?;
    }

    let op_file = match crate::graphql::schema::GraphqlOperationFile::load(op_abs) {
        Ok(v) => v,
        Err(err) => {
            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
            status = "failed".to_string();
            message = Some("graphql_runner_failed".to_string());
            return Ok(GraphqlCaseRunOutput {
                status,
                message,
                assertions,
                command_snippet: None,
                stdout_path,
                stderr_path,
                skip_cleanup: true,
            });
        }
    };

    if status != "failed" {
        match crate::graphql::runner::execute_graphql_request(
            &endpoint_url,
            bearer.as_deref(),
            &op_file.operation,
            vars_json.as_ref(),
        ) {
            Ok(executed) => {
                write_file(&stdout_path, &executed.response.body)?;
                let response_json: serde_json::Value =
                    serde_json::from_slice(&executed.response.body)
                        .unwrap_or(serde_json::Value::Null);

                let expect_jq = expect_jq_raw.trim();
                let expect_jq = (!expect_jq.is_empty()).then_some(expect_jq);

                let a = crate::graphql::expect::evaluate_graphql_response_for_suite(
                    &response_json,
                    allow_errors,
                    expect_jq,
                )
                .unwrap_or(crate::graphql::expect::GraphqlAssertions {
                    default_no_errors: "failed".to_string(),
                    default_has_data: None,
                    jq: None,
                });
                assertions = Some(a.to_json());

                let jq_failed = a.jq.as_deref() == Some("failed");
                let has_data_failed = a.default_has_data.as_deref() == Some("failed");
                let no_errors_failed = a.default_no_errors == "failed";

                if no_errors_failed && !allow_errors {
                    status = "failed".to_string();
                    message = Some("graphql_errors_present".to_string());
                } else if has_data_failed {
                    status = "failed".to_string();
                    message = Some("graphql_data_missing_or_null".to_string());
                } else if jq_failed {
                    status = "failed".to_string();
                    message = Some("expect_jq_failed".to_string());
                } else {
                    status = "passed".to_string();
                }
            }
            Err(err) => {
                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                status = "failed".to_string();
                message = Some("graphql_runner_failed".to_string());
            }
        }
    }

    let mut argv: Vec<String> = vec![
        "api-gql".to_string(),
        "call".to_string(),
        "--config-dir".to_string(),
        gql_config_dir.to_string(),
    ];
    if effective_no_history {
        argv.push("--no-history".to_string());
    }
    if !gql_url.trim().is_empty() {
        argv.push("--url".to_string());
        argv.push(gql_url.to_string());
    } else if !effective_env.trim().is_empty() {
        argv.push("--env".to_string());
        argv.push(effective_env.to_string());
    }
    if !gql_jwt.trim().is_empty() && access_token_for_case.trim().is_empty() {
        argv.push("--jwt".to_string());
        argv.push(gql_jwt.to_string());
    }
    argv.push(path_relative_to_repo_or_abs(repo_root, op_abs));
    if let Some(vp) = vars_abs {
        argv.push(path_relative_to_repo_or_abs(repo_root, vp));
    }

    let args = super::mask_args_for_command_snippet(&argv[1..]);
    let env_prefix = if !access_token_for_case.trim().is_empty() {
        "ACCESS_TOKEN=REDACTED REST_TOKEN_NAME= GQL_JWT_NAME="
    } else {
        ""
    };
    let snippet = if env_prefix.is_empty() {
        format!("{} {}", cli_util::shell_quote("api-gql"), args)
    } else {
        format!("{env_prefix} {} {}", cli_util::shell_quote("api-gql"), args)
    };

    Ok(GraphqlCaseRunOutput {
        status,
        message,
        assertions,
        command_snippet: Some(snippet.trim().to_string()),
        stdout_path,
        stderr_path,
        skip_cleanup: false,
    })
}
