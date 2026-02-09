use anyhow::Context;

use crate::Result;
use crate::suite::resolve::{resolve_path_from_repo_root, write_file};
use crate::suite::runtime::{resolve_rest_base_url, resolve_rest_token_profile};
use crate::suite::schema::SuiteCleanupStep;

use super::template::{parse_vars_map, render_template};
use super::{CleanupContext, append_log, log_failure_with_stderr_file};

pub(super) fn rest_cleanup_step(
    ctx: &mut CleanupContext<'_>,
    response_json: &serde_json::Value,
    step: &SuiteCleanupStep,
    step_index: usize,
) -> Result<bool> {
    let method = step.method.trim();
    let method = if method.is_empty() {
        "DELETE".to_string()
    } else {
        method.to_ascii_uppercase()
    };

    let path_template = step.path_template.trim();
    if path_template.is_empty() {
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(rest) render failed: step[{step_index}] pathTemplate=<missing>"),
        )?;
        return Ok(false);
    }

    let vars_map = parse_vars_map(step.vars.as_ref())?;
    let cleanup_path = render_template(path_template, response_json, &vars_map).ok();
    let cleanup_path = cleanup_path.unwrap_or_default().trim().to_string();
    if cleanup_path.is_empty() {
        append_log(
            ctx.main_stderr_file,
            &format!(
                "cleanup(rest) render failed: step[{step_index}] pathTemplate={path_template}"
            ),
        )?;
        return Ok(false);
    }
    if !cleanup_path.starts_with('/') {
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(rest) invalid path: step[{step_index}] {cleanup_path}"),
        )?;
        return Ok(false);
    }

    let mut expect_status = step
        .expect
        .as_ref()
        .and_then(|e| e.status)
        .or(step.expect_status);
    if expect_status.is_none() {
        expect_status = Some(if method == "DELETE" { 204 } else { 200 });
    }
    let expect_status = expect_status.unwrap_or(200);

    let expect_jq = if !step.expect_jq.trim().is_empty() {
        step.expect_jq.trim().to_string()
    } else {
        step.expect
            .as_ref()
            .map(|e| e.jq.trim().to_string())
            .unwrap_or_default()
    };

    let cleanup_config_dir = if !step.config_dir.trim().is_empty() {
        step.config_dir.trim()
    } else if !ctx.rest_config_dir.trim().is_empty() {
        ctx.rest_config_dir.trim()
    } else {
        ctx.suite_defaults.rest.config_dir.trim()
    };

    let cleanup_url = if !step.url.trim().is_empty() {
        step.url.trim()
    } else if !ctx.rest_url.trim().is_empty() {
        ctx.rest_url.trim()
    } else {
        ""
    };

    let cleanup_env = if !step.env.trim().is_empty() {
        step.env.trim()
    } else {
        ctx.effective_env
    };

    let base_url = match resolve_rest_base_url(
        ctx.repo_root,
        cleanup_config_dir,
        cleanup_url,
        cleanup_env,
        ctx.suite_defaults,
        ctx.env_rest_url,
    ) {
        Ok(v) => v,
        Err(err) => {
            append_log(
                ctx.main_stderr_file,
                &format!("cleanup(rest) failed: step[{step_index}] {err}"),
            )?;
            return Ok(false);
        }
    };

    let mut cleanup_token = if !step.token.trim().is_empty() {
        step.token.trim().to_string()
    } else if !ctx.rest_token.trim().is_empty() {
        ctx.rest_token.trim().to_string()
    } else if !ctx.gql_jwt.trim().is_empty() {
        ctx.gql_jwt.trim().to_string()
    } else {
        ctx.suite_defaults.rest.token.trim().to_string()
    };
    cleanup_token = cleanup_token.trim().to_string();

    let mut access_token = ctx.access_token_for_case.trim().to_string();
    if let Some(auth) = ctx.auth_manager.as_deref_mut()
        && !cleanup_token.is_empty()
    {
        match auth.ensure_token(
            &cleanup_token,
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
                            "cleanup(rest) auth failed: step[{step_index}] profile={cleanup_token}"
                        ),
                    )?;
                    append_log(ctx.main_stderr_file, &err)?;
                    return Ok(false);
                }
            }
        }
    }

    let setup_dir_abs = resolve_path_from_repo_root(ctx.repo_root, cleanup_config_dir);
    let bearer_token = if !access_token.is_empty() {
        Some(access_token)
    } else if !cleanup_token.is_empty() {
        match resolve_rest_token_profile(&setup_dir_abs, &cleanup_token) {
            Ok(t) => Some(t),
            Err(err) => {
                append_log(
                    ctx.main_stderr_file,
                    &format!("cleanup(rest) failed: step[{step_index}] {err}"),
                )?;
                return Ok(false);
            }
        }
    } else {
        None
    };

    let mut request_obj = serde_json::Map::new();
    request_obj.insert(
        "method".to_string(),
        serde_json::Value::String(method.clone()),
    );
    request_obj.insert(
        "path".to_string(),
        serde_json::Value::String(cleanup_path.clone()),
    );
    let mut expect_obj = serde_json::Map::new();
    expect_obj.insert(
        "status".to_string(),
        serde_json::Value::Number(expect_status.into()),
    );
    if !expect_jq.trim().is_empty() {
        expect_obj.insert(
            "jq".to_string(),
            serde_json::Value::String(expect_jq.clone()),
        );
    }
    request_obj.insert("expect".to_string(), serde_json::Value::Object(expect_obj));
    let request_json = serde_json::Value::Object(request_obj);

    let cleanup_request_file = ctx
        .run_dir
        .join(format!("{}.cleanup.{step_index}.request.json", ctx.safe_id));
    write_file(
        &cleanup_request_file,
        serde_json::to_vec_pretty(&request_json)?.as_slice(),
    )?;

    let cleanup_stdout_file = ctx.run_dir.join(format!(
        "{}.cleanup.{step_index}.response.json",
        ctx.safe_id
    ));
    let cleanup_stderr_file = ctx
        .run_dir
        .join(format!("{}.cleanup.{step_index}.stderr.log", ctx.safe_id));
    write_file(&cleanup_stdout_file, b"")?;
    write_file(&cleanup_stderr_file, b"")?;

    let request = crate::rest::schema::parse_rest_request_json(request_json)
        .context("parse cleanup request")?;
    let request_file = crate::rest::schema::RestRequestFile {
        path: cleanup_request_file.clone(),
        request,
    };

    let executed = match crate::rest::runner::execute_rest_request(
        &request_file,
        &base_url,
        bearer_token.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            write_file(&cleanup_stderr_file, format!("{err:#}\n").as_bytes())?;
            log_failure_with_stderr_file(
                ctx.main_stderr_file,
                &format!("cleanup(rest) failed: step[{step_index}] rc=1 {method} {cleanup_path}"),
                &cleanup_stderr_file,
            )?;
            return Ok(false);
        }
    };

    write_file(&cleanup_stdout_file, &executed.response.body)?;

    if let Err(err) = crate::rest::expect::evaluate_main_response(&request_file.request, &executed)
    {
        write_file(&cleanup_stderr_file, format!("{err:#}\n").as_bytes())?;
        log_failure_with_stderr_file(
            ctx.main_stderr_file,
            &format!("cleanup(rest) failed: step[{step_index}] rc=1 {method} {cleanup_path}"),
            &cleanup_stderr_file,
        )?;
        return Ok(false);
    }

    Ok(true)
}
