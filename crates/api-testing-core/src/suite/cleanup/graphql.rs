use anyhow::Context;

use crate::suite::resolve::{resolve_path_from_repo_root, write_file};
use crate::suite::runtime::{resolve_gql_url, resolve_graphql_bearer_token};
use crate::suite::schema::SuiteCleanupStep;
use crate::Result;

use super::template::{parse_vars_map, render_template};
use super::{append_log, log_failure_with_error, read_json_file, CleanupContext};

pub(super) fn graphql_cleanup_step(
    ctx: &mut CleanupContext<'_>,
    response_json: &serde_json::Value,
    step: &SuiteCleanupStep,
    step_index: usize,
) -> Result<bool> {
    let op = step.op.trim();
    if op.is_empty() {
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(graphql) runner failed: step[{step_index}] op=<missing>"),
        )?;
        return Ok(false);
    }
    let op_abs = resolve_path_from_repo_root(ctx.repo_root, op);
    if !op_abs.is_file() {
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(graphql) runner failed: step[{step_index}] op not found: {op}"),
        )?;
        return Ok(false);
    }

    let cleanup_config_dir = if !step.config_dir.trim().is_empty() {
        step.config_dir.trim()
    } else if !ctx.gql_config_dir.trim().is_empty() {
        ctx.gql_config_dir.trim()
    } else {
        ctx.suite_defaults.graphql.config_dir.trim()
    };

    let cleanup_jwt = if !step.jwt.trim().is_empty() {
        step.jwt.trim().to_string()
    } else if !ctx.gql_jwt.trim().is_empty() {
        ctx.gql_jwt.trim().to_string()
    } else if !ctx.rest_token.trim().is_empty() {
        ctx.rest_token.trim().to_string()
    } else {
        ctx.suite_defaults.graphql.jwt.trim().to_string()
    };

    let cleanup_url_override = if !step.url.trim().is_empty() {
        step.url.trim()
    } else if !ctx.gql_url.trim().is_empty() {
        ctx.gql_url.trim()
    } else {
        ""
    };

    let cleanup_env = if !step.env.trim().is_empty() {
        step.env.trim()
    } else {
        ctx.effective_env
    };

    let endpoint_url = match resolve_gql_url(
        ctx.repo_root,
        cleanup_config_dir,
        cleanup_url_override,
        cleanup_env,
        ctx.suite_defaults,
        ctx.env_gql_url,
    ) {
        Ok(v) => v,
        Err(err) => {
            append_log(
                ctx.main_stderr_file,
                &format!("cleanup(graphql) runner failed: step[{step_index}] {err}"),
            )?;
            return Ok(false);
        }
    };

    let allow_errors = step.allow_errors;
    let expect_jq = if !step.expect_jq.trim().is_empty() {
        step.expect_jq.trim().to_string()
    } else {
        step.expect
            .as_ref()
            .map(|e| e.jq.trim().to_string())
            .unwrap_or_default()
    };
    if allow_errors && expect_jq.trim().is_empty() {
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(graphql) expect.jq missing: step[{step_index}]"),
        )?;
        return Ok(false);
    }

    let mut access_token = ctx.access_token_for_case.trim().to_string();
    if let Some(auth) = ctx.auth_manager.as_deref_mut()
        && !cleanup_jwt.trim().is_empty()
    {
        match auth.ensure_token(
            &cleanup_jwt,
            ctx.repo_root,
            ctx.suite_defaults,
            ctx.env_rest_url,
            ctx.env_gql_url,
        ) {
            Ok(t) => access_token = t,
            Err(err) => {
                if access_token.is_empty() {
                    append_log(
                            ctx.main_stderr_file,
                            &format!(
                                "cleanup(graphql) auth failed: step[{step_index}] profile={cleanup_jwt}"
                            ),
                        )?;
                    append_log(ctx.main_stderr_file, &err)?;
                    return Ok(false);
                }
            }
        }
    }

    let setup_dir_abs = resolve_path_from_repo_root(ctx.repo_root, cleanup_config_dir);
    let mut auth_stderr: Vec<u8> = Vec::new();
    let bearer_token = if !access_token.is_empty() {
        Some(access_token)
    } else {
        match resolve_graphql_bearer_token(
            &setup_dir_abs,
            &endpoint_url,
            &op_abs,
            &cleanup_jwt,
            &mut auth_stderr,
        ) {
            Ok(v) => v,
            Err(err) => {
                append_log(
                    ctx.main_stderr_file,
                    &format!("cleanup(graphql) runner failed: step[{step_index}] {err}"),
                )?;
                return Ok(false);
            }
        }
    };

    let vars_json: Option<serde_json::Value> = if !step.vars_jq.trim().is_empty() {
        let out = crate::jq::query(response_json, step.vars_jq.trim()).ok();
        let out = out.unwrap_or_default();
        if out.len() != 1 || !out[0].is_object() {
            append_log(
                ctx.main_stderr_file,
                &format!(
                    "cleanup(graphql) varsJq failed: step[{step_index}] varsJq={}",
                    step.vars_jq.trim()
                ),
            )?;
            return Ok(false);
        }
        Some(out[0].clone())
    } else if !step.vars_template.trim().is_empty() {
        let template_path = resolve_path_from_repo_root(ctx.repo_root, step.vars_template.trim());
        if !template_path.is_file() {
            append_log(
                ctx.main_stderr_file,
                &format!(
                    "cleanup(graphql) varsTemplate render failed: step[{step_index}] template={}",
                    step.vars_template.trim()
                ),
            )?;
            return Ok(false);
        }
        let template_text = std::fs::read_to_string(&template_path)
            .with_context(|| format!("read varsTemplate: {}", template_path.display()))?;

        let vars_map = parse_vars_map(step.vars.as_ref())?;
        let rendered = match render_template(&template_text, response_json, &vars_map) {
            Ok(v) => v,
            Err(_) => {
                append_log(
                    ctx.main_stderr_file,
                    &format!(
                        "cleanup(graphql) varsTemplate render failed: step[{step_index}] template={}",
                        step.vars_template.trim()
                    ),
                )?;
                return Ok(false);
            }
        };
        let v: serde_json::Value = serde_json::from_str(&rendered)
            .map_err(|_| anyhow::anyhow!("cleanup(graphql) varsTemplate rendered invalid JSON"))?;
        if !v.is_object() {
            append_log(
                ctx.main_stderr_file,
                &format!(
                    "cleanup(graphql) varsTemplate render failed: step[{step_index}] template={}",
                    step.vars_template.trim()
                ),
            )?;
            return Ok(false);
        }
        Some(v)
    } else if let Some(v) = step.vars.as_ref() {
        if v.is_null() {
            None
        } else if let Some(path) = v.as_str() {
            let abs = resolve_path_from_repo_root(ctx.repo_root, path);
            if !abs.is_file() {
                append_log(
                    ctx.main_stderr_file,
                    &format!("cleanup(graphql) vars not found: step[{step_index}] {path}"),
                )?;
                return Ok(false);
            }
            Some(read_json_file(&abs)?)
        } else {
            append_log(
                ctx.main_stderr_file,
                "cleanup(graphql) vars must be a file path string",
            )?;
            return Ok(false);
        }
    } else {
        None
    };

    let cleanup_stdout_file = ctx.run_dir.join(format!(
        "{}.cleanup.{step_index}.response.json",
        ctx.safe_id
    ));
    let cleanup_stderr_file = ctx
        .run_dir
        .join(format!("{}.cleanup.{step_index}.stderr.log", ctx.safe_id));
    write_file(&cleanup_stdout_file, b"")?;
    write_file(&cleanup_stderr_file, b"")?;

    if !auth_stderr.is_empty() {
        write_file(&cleanup_stderr_file, &auth_stderr)?;
    }

    let op_file = crate::graphql::schema::GraphqlOperationFile::load(&op_abs)
        .context("load graphql cleanup op")?;

    let executed = match crate::graphql::runner::execute_graphql_request(
        &endpoint_url,
        bearer_token.as_deref(),
        &op_file.operation,
        vars_json.as_ref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            log_failure_with_error(
                ctx.main_stderr_file,
                &format!("cleanup(graphql) runner failed: step[{step_index}] rc=1 op={op}"),
                &err,
            )?;
            return Ok(false);
        }
    };

    write_file(&cleanup_stdout_file, &executed.response.body)?;

    let response_json2: serde_json::Value = match serde_json::from_slice(&executed.response.body) {
        Ok(v) => v,
        Err(_) => {
            append_log(
                ctx.main_stderr_file,
                &format!("cleanup(graphql) runner failed: step[{step_index}] invalid JSON"),
            )?;
            return Ok(false);
        }
    };

    let errors_present = response_json2
        .get("errors")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if errors_present && !allow_errors {
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(graphql) errors present: step[{step_index}] op={op}"),
        )?;
        return Ok(false);
    }

    if !expect_jq.trim().is_empty() {
        let ok = crate::jq::eval_exit_status(&response_json2, expect_jq.trim()).unwrap_or(false);
        if !ok {
            append_log(
                ctx.main_stderr_file,
                &format!(
                    "cleanup(graphql) expect.jq failed: step[{step_index}] {}",
                    expect_jq.trim()
                ),
            )?;
            return Ok(false);
        }
    }

    Ok(true)
}
