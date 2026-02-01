use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::Context;

use crate::suite::auth::SuiteAuthManager;
use crate::suite::resolve::{
    resolve_gql_url_for_env, resolve_path_from_repo_root, resolve_rest_base_url_for_env, write_file,
};
use crate::suite::safety::writes_enabled;
use crate::suite::schema::{SuiteCleanup, SuiteCleanupStep, SuiteDefaults};
use crate::Result;

pub struct CleanupContext<'a> {
    pub repo_root: &'a Path,
    pub run_dir: &'a Path,
    pub case_id: &'a str,
    pub safe_id: &'a str,

    pub main_response_file: Option<&'a Path>,
    pub main_stderr_file: &'a Path,

    pub allow_writes_flag: bool,
    pub effective_env: &'a str,
    pub effective_no_history: bool,

    pub suite_defaults: &'a SuiteDefaults,
    pub env_rest_url: &'a str,
    pub env_gql_url: &'a str,

    pub rest_config_dir: &'a str,
    pub rest_url: &'a str,
    pub rest_token: &'a str,

    pub gql_config_dir: &'a str,
    pub gql_url: &'a str,
    pub gql_jwt: &'a str,

    pub access_token_for_case: &'a str,
    pub auth_manager: Option<&'a mut SuiteAuthManager>,

    pub cleanup: Option<&'a SuiteCleanup>,
}

fn append_log(path: &Path, line: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open log for append: {}", path.display()))?;
    writeln!(f, "{line}").context("append log line")?;
    Ok(())
}

fn read_json_file(path: &Path) -> Result<serde_json::Value> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read JSON file: {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    Ok(v)
}

fn jq_extract_first_string(value: &serde_json::Value, expr: &str) -> Option<String> {
    let lines = crate::jq::query_raw(value, expr).ok()?;
    let first = lines.into_iter().next()?.trim().to_string();
    if first.is_empty() || first == "null" {
        None
    } else {
        Some(first)
    }
}

fn parse_vars_map(vars: Option<&serde_json::Value>) -> Result<BTreeMap<String, String>> {
    let Some(vars) = vars else {
        return Ok(BTreeMap::new());
    };
    if vars.is_null() {
        return Ok(BTreeMap::new());
    }
    let obj = vars.as_object().context("cleanup.vars must be an object")?;
    let mut out = BTreeMap::new();
    for (k, v) in obj {
        let Some(expr) = v.as_str() else {
            anyhow::bail!("cleanup.vars values must be strings");
        };
        out.insert(k.clone(), expr.to_string());
    }
    Ok(out)
}

fn render_template(
    template: &str,
    response_json: &serde_json::Value,
    vars: &BTreeMap<String, String>,
) -> Result<String> {
    let mut out = template.to_string();
    for (key, expr) in vars {
        let Some(value) = jq_extract_first_string(response_json, expr) else {
            anyhow::bail!("template var '{key}' failed to resolve");
        };
        out = out.replace(&format!("{{{{{key}}}}}"), &value);
    }
    Ok(out)
}

fn resolve_rest_base_url(
    repo_root: &Path,
    setup_dir_raw: &str,
    url_override: &str,
    env_value: &str,
    suite_defaults: &SuiteDefaults,
    env_rest_url: &str,
) -> Result<String> {
    let url_override = url_override.trim();
    if !url_override.is_empty() {
        return Ok(url_override.to_string());
    }
    let default_url = suite_defaults.rest.url.trim();
    if !default_url.is_empty() {
        return Ok(default_url.to_string());
    }
    let env_rest_url = env_rest_url.trim();
    if !env_rest_url.is_empty() {
        return Ok(env_rest_url.to_string());
    }
    let env_value = env_value.trim();
    if !env_value.is_empty() {
        let setup_dir = resolve_path_from_repo_root(repo_root, setup_dir_raw);
        return resolve_rest_base_url_for_env(&setup_dir, env_value);
    }
    Ok("http://localhost:6700".to_string())
}

fn resolve_gql_url(
    repo_root: &Path,
    setup_dir_raw: &str,
    url_override: &str,
    env_value: &str,
    suite_defaults: &SuiteDefaults,
    env_gql_url: &str,
) -> Result<String> {
    let url_override = url_override.trim();
    if !url_override.is_empty() {
        return Ok(url_override.to_string());
    }
    let default_url = suite_defaults.graphql.url.trim();
    if !default_url.is_empty() {
        return Ok(default_url.to_string());
    }
    let env_gql_url = env_gql_url.trim();
    if !env_gql_url.is_empty() {
        return Ok(env_gql_url.to_string());
    }
    let env_value = env_value.trim();
    if !env_value.is_empty() {
        let setup_dir = resolve_path_from_repo_root(repo_root, setup_dir_raw);
        return resolve_gql_url_for_env(&setup_dir, env_value);
    }
    Ok("http://localhost:6700/graphql".to_string())
}

fn resolve_rest_token_profile(setup_dir: &Path, profile: &str) -> Result<String> {
    let tokens_env = setup_dir.join("tokens.env");
    let tokens_local = setup_dir.join("tokens.local.env");
    let files: Vec<&Path> = if tokens_env.is_file() || tokens_local.is_file() {
        vec![&tokens_env, &tokens_local]
    } else {
        Vec::new()
    };

    let key = profile.trim().to_ascii_uppercase();
    let mut env_key = String::new();
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            env_key.push(c);
        } else if !env_key.ends_with('_') {
            env_key.push('_');
        }
    }
    while env_key.ends_with('_') {
        env_key.pop();
    }

    let var = format!("REST_TOKEN_{env_key}");
    let found = crate::env_file::read_var_last_wins(&var, &files)?;
    found.ok_or_else(|| anyhow::anyhow!("Token profile '{profile}' is empty/missing."))
}

fn resolve_graphql_bearer_token(
    setup_dir: &Path,
    endpoint_url: &str,
    operation_file: &Path,
    jwt_profile: &str,
    stderr: &mut dyn Write,
) -> Result<Option<String>> {
    if jwt_profile.trim().is_empty() {
        return Ok(None);
    }
    let auth = crate::graphql::auth::resolve_bearer_token(
        setup_dir,
        endpoint_url,
        Some(operation_file),
        Some(jwt_profile),
        stderr,
    )?;
    Ok(auth.bearer_token)
}

fn cleanup_step_type(raw: &str) -> String {
    let t = raw.trim().to_ascii_lowercase();
    if t == "gql" {
        "graphql".to_string()
    } else {
        t
    }
}

fn rest_cleanup_step(
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
    if let Some(auth) = ctx.auth_manager.as_deref_mut() {
        if !cleanup_token.is_empty() {
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
            append_log(
                ctx.main_stderr_file,
                &format!("cleanup(rest) failed: step[{step_index}] rc=1 {method} {cleanup_path}"),
            )?;
            let stderr_text = std::fs::read_to_string(&cleanup_stderr_file).unwrap_or_default();
            for line in stderr_text.lines() {
                append_log(ctx.main_stderr_file, line)?;
            }
            return Ok(false);
        }
    };

    write_file(&cleanup_stdout_file, &executed.response.body)?;

    if let Err(err) = crate::rest::expect::evaluate_main_response(&request_file.request, &executed)
    {
        write_file(&cleanup_stderr_file, format!("{err:#}\n").as_bytes())?;
        append_log(
            ctx.main_stderr_file,
            &format!("cleanup(rest) failed: step[{step_index}] rc=1 {method} {cleanup_path}"),
        )?;
        let stderr_text = std::fs::read_to_string(&cleanup_stderr_file).unwrap_or_default();
        for line in stderr_text.lines() {
            append_log(ctx.main_stderr_file, line)?;
        }
        return Ok(false);
    }

    Ok(true)
}

fn graphql_cleanup_step(
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
    if let Some(auth) = ctx.auth_manager.as_deref_mut() {
        if !cleanup_jwt.trim().is_empty() {
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
            append_log(
                ctx.main_stderr_file,
                &format!("cleanup(graphql) runner failed: step[{step_index}] rc=1 op={op}"),
            )?;
            append_log(ctx.main_stderr_file, &format!("{err:#}"))?;
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

pub fn run_case_cleanup(ctx: &mut CleanupContext<'_>) -> Result<bool> {
    let Some(cleanup) = ctx.cleanup else {
        return Ok(true);
    };

    if !writes_enabled(ctx.allow_writes_flag, ctx.effective_env) {
        append_log(
            ctx.main_stderr_file,
            "cleanup skipped (writes disabled): enable with API_TEST_ALLOW_WRITES_ENABLED=true (or --allow-writes)",
        )?;
        return Ok(true);
    }

    let Some(main_response_file) = ctx.main_response_file else {
        append_log(
            ctx.main_stderr_file,
            "cleanup failed: missing main response file",
        )?;
        return Ok(false);
    };
    if !main_response_file.is_file() {
        append_log(
            ctx.main_stderr_file,
            "cleanup failed: missing main response file",
        )?;
        return Ok(false);
    }

    let response_json = read_json_file(main_response_file)?;

    let mut any_failed = false;
    for (i, step) in cleanup.steps().iter().enumerate() {
        let ty = cleanup_step_type(&step.step_type);
        let ok = match ty.as_str() {
            "rest" => rest_cleanup_step(ctx, &response_json, step, i)?,
            "graphql" => graphql_cleanup_step(ctx, &response_json, step, i)?,
            other => {
                append_log(
                    ctx.main_stderr_file,
                    &format!("cleanup failed: unknown step type: {other}"),
                )?;
                false
            }
        };
        if !ok {
            any_failed = true;
        }
    }

    Ok(!any_failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::fixtures::{GraphqlSetupFixture, RestSetupFixture, SuiteFixture};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    #[test]
    fn suite_cleanup_step_type_maps_gql_to_graphql() {
        assert_eq!(cleanup_step_type("gql"), "graphql");
        assert_eq!(cleanup_step_type(" GQL "), "graphql");
        assert_eq!(cleanup_step_type("REST"), "rest");
    }

    #[test]
    fn suite_cleanup_parse_vars_map_none_and_null_are_empty() {
        assert_eq!(parse_vars_map(None).unwrap(), BTreeMap::new());

        let v = serde_json::Value::Null;
        assert_eq!(parse_vars_map(Some(&v)).unwrap(), BTreeMap::new());
    }

    #[test]
    fn suite_cleanup_parse_vars_map_validates_object_and_string_values() {
        let v = serde_json::json!(["x"]);
        let err = parse_vars_map(Some(&v)).unwrap_err();
        assert!(err.to_string().contains("cleanup.vars must be an object"));

        let v = serde_json::json!({"id": 1});
        let err = parse_vars_map(Some(&v)).unwrap_err();
        assert!(err
            .to_string()
            .contains("cleanup.vars values must be strings"));
    }

    #[test]
    fn suite_cleanup_rest_base_url_resolution_precedence() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();
        let defaults = SuiteDefaults::default();

        assert_eq!(
            resolve_rest_base_url(
                repo_root,
                "setup/rest",
                "https://override.example",
                "staging",
                &defaults,
                "",
            )
            .unwrap(),
            "https://override.example"
        );

        let mut defaults2 = SuiteDefaults::default();
        defaults2.rest.url = "https://defaults.example".to_string();
        assert_eq!(
            resolve_rest_base_url(repo_root, "setup/rest", "", "staging", &defaults2, "").unwrap(),
            "https://defaults.example"
        );

        assert_eq!(
            resolve_rest_base_url(
                repo_root,
                "setup/rest",
                "",
                "staging",
                &defaults,
                "https://env.example",
            )
            .unwrap(),
            "https://env.example"
        );

        std::fs::create_dir_all(repo_root.join("setup/rest")).unwrap();
        std::fs::write(
            repo_root.join("setup/rest/endpoints.env"),
            "REST_URL_STAGING=https://fromfile.example\n",
        )
        .unwrap();
        assert_eq!(
            resolve_rest_base_url(repo_root, "setup/rest", "", "staging", &defaults, "").unwrap(),
            "https://fromfile.example"
        );
    }

    #[test]
    fn suite_cleanup_gql_url_resolution_precedence() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();
        let defaults = SuiteDefaults::default();

        assert_eq!(
            resolve_gql_url(
                repo_root,
                "setup/graphql",
                "https://override.example/graphql",
                "staging",
                &defaults,
                "",
            )
            .unwrap(),
            "https://override.example/graphql"
        );

        let mut defaults2 = SuiteDefaults::default();
        defaults2.graphql.url = "https://defaults.example/graphql".to_string();
        assert_eq!(
            resolve_gql_url(repo_root, "setup/graphql", "", "staging", &defaults2, "").unwrap(),
            "https://defaults.example/graphql"
        );

        assert_eq!(
            resolve_gql_url(
                repo_root,
                "setup/graphql",
                "",
                "staging",
                &defaults,
                "https://env.example/graphql",
            )
            .unwrap(),
            "https://env.example/graphql"
        );

        std::fs::create_dir_all(repo_root.join("setup/graphql")).unwrap();
        std::fs::write(
            repo_root.join("setup/graphql/endpoints.env"),
            "GQL_URL_STAGING=https://fromfile.example/graphql\n",
        )
        .unwrap();
        assert_eq!(
            resolve_gql_url(repo_root, "setup/graphql", "", "staging", &defaults, "").unwrap(),
            "https://fromfile.example/graphql"
        );
    }

    #[test]
    fn suite_cleanup_rest_url_selection_uses_rest_endpoints_env() {
        let fixture = RestSetupFixture::new();
        fixture.write_endpoints_env("REST_URL_STAGING=https://fromfile.example\n");
        let defaults = SuiteDefaults::default();

        let url = resolve_rest_base_url(&fixture.root, "setup/rest", "", "staging", &defaults, "")
            .unwrap();

        assert_eq!(url, "https://fromfile.example");
    }

    #[test]
    fn suite_cleanup_graphql_url_selection_uses_graphql_endpoints_env() {
        let fixture = GraphqlSetupFixture::new();
        fixture.write_endpoints_env("GQL_URL_STAGING=https://fromfile.example/graphql\n");
        let defaults = SuiteDefaults::default();

        let url =
            resolve_gql_url(&fixture.root, "setup/graphql", "", "staging", &defaults, "").unwrap();

        assert_eq!(url, "https://fromfile.example/graphql");
    }

    #[test]
    fn suite_cleanup_template_replaces_vars() {
        let response = serde_json::json!({"data": {"id": "123"}});
        let mut vars = BTreeMap::new();
        vars.insert("id".to_string(), ".data.id".to_string());
        let out = render_template("/items/{{id}}", &response, &vars).unwrap();
        assert_eq!(out, "/items/123");
    }

    #[test]
    fn suite_cleanup_template_missing_var_is_error() {
        let response = serde_json::json!({"data": {"id": "123"}});
        let mut vars = BTreeMap::new();
        vars.insert("id".to_string(), ".data.missing".to_string());
        let err = render_template("/items/{{id}}", &response, &vars).unwrap_err();
        assert!(err
            .to_string()
            .contains("template var 'id' failed to resolve"));
    }

    #[test]
    fn suite_cleanup_disabled_writes_is_noop_with_log() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let cleanup = SuiteCleanup::One(Box::new(SuiteCleanupStep {
            step_type: "rest".to_string(),
            config_dir: String::new(),
            url: String::new(),
            env: String::new(),
            no_history: None,
            method: "DELETE".to_string(),
            path_template: "/x".to_string(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: false,
        }));

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: false,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: Some(&cleanup),
        };

        assert!(run_case_cleanup(&mut ctx).unwrap());
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("cleanup skipped"));
    }

    #[test]
    fn suite_cleanup_rest_step_missing_path_template_is_early_failure_with_log() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({});
        let step = SuiteCleanupStep {
            step_type: "rest".to_string(),
            config_dir: String::new(),
            url: String::new(),
            env: String::new(),
            no_history: None,
            method: "DELETE".to_string(),
            path_template: String::new(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: false,
        };

        let ok = rest_cleanup_step(&mut ctx, &response_json, &step, 0).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("pathTemplate=<missing>"));
    }

    #[test]
    fn suite_cleanup_rest_step_invalid_path_is_early_failure_with_log() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({"data": {"id": "123"}});
        let step = SuiteCleanupStep {
            step_type: "rest".to_string(),
            config_dir: String::new(),
            url: "https://override.example".to_string(),
            env: String::new(),
            no_history: None,
            method: "DELETE".to_string(),
            path_template: "invalid".to_string(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: false,
        };

        let ok = rest_cleanup_step(&mut ctx, &response_json, &step, 1).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("invalid path"));
    }

    #[test]
    fn suite_cleanup_graphql_step_missing_op_is_early_failure_with_log() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({});
        let step = SuiteCleanupStep {
            step_type: "graphql".to_string(),
            config_dir: String::new(),
            url: "https://override.example/graphql".to_string(),
            env: String::new(),
            no_history: None,
            method: String::new(),
            path_template: String::new(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: false,
        };

        let ok = graphql_cleanup_step(&mut ctx, &response_json, &step, 0).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("op=<missing>"));
    }

    #[test]
    fn suite_cleanup_graphql_step_vars_jq_failure_is_logged() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let op_dir = repo.join("ops");
        std::fs::create_dir_all(&op_dir).unwrap();
        let op_path = op_dir.join("cleanup.graphql");
        std::fs::write(&op_path, "query Q { ok }\n").unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({"data": {"id": "123"}});
        let step = SuiteCleanupStep {
            step_type: "graphql".to_string(),
            config_dir: String::new(),
            url: "https://override.example/graphql".to_string(),
            env: String::new(),
            no_history: None,
            method: String::new(),
            path_template: String::new(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: op_path.to_string_lossy().to_string(),
            vars_jq: "???".to_string(),
            vars_template: String::new(),
            allow_errors: false,
        };

        let ok = graphql_cleanup_step(&mut ctx, &response_json, &step, 1).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("varsJq failed"));
    }

    #[test]
    fn suite_cleanup_graphql_step_invalid_vars_template_vars_map_is_error() {
        let fixture = GraphqlSetupFixture::new();
        let run_dir = fixture.root.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let op_path = fixture.root.join("ops/cleanup.graphql");
        std::fs::create_dir_all(op_path.parent().unwrap()).unwrap();
        std::fs::write(&op_path, "query Q { ok }\n").unwrap();

        let template_path = fixture.root.join("templates/vars.json");
        std::fs::create_dir_all(template_path.parent().unwrap()).unwrap();
        std::fs::write(&template_path, r#"{"id":"{{id}}"}"#).unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: &fixture.root,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({"data": {"id": "123"}});
        let step = SuiteCleanupStep {
            step_type: "graphql".to_string(),
            config_dir: String::new(),
            url: "https://override.example/graphql".to_string(),
            env: String::new(),
            no_history: None,
            method: String::new(),
            path_template: String::new(),
            vars: Some(serde_json::json!(["bad"])),
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: op_path
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_string_lossy()
                .to_string(),
            vars_jq: String::new(),
            vars_template: template_path
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_string_lossy()
                .to_string(),
            allow_errors: false,
        };

        let err = graphql_cleanup_step(&mut ctx, &response_json, &step, 2).unwrap_err();
        assert!(err.to_string().contains("cleanup.vars must be an object"));
    }

    #[test]
    fn suite_cleanup_graphql_step_vars_template_render_failure_is_logged() {
        let fixture = SuiteFixture::new();
        let run_dir = fixture.root.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let op_path = fixture.root.join("ops/cleanup.graphql");
        std::fs::create_dir_all(op_path.parent().unwrap()).unwrap();
        std::fs::write(&op_path, "query Q { ok }\n").unwrap();

        let template_path = fixture.root.join("templates/vars.json");
        std::fs::create_dir_all(template_path.parent().unwrap()).unwrap();
        std::fs::write(&template_path, r#"{"id":"{{id}}"}"#).unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: &fixture.root,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({"data": {"other": "x"}});
        let step = SuiteCleanupStep {
            step_type: "graphql".to_string(),
            config_dir: String::new(),
            url: "https://override.example/graphql".to_string(),
            env: String::new(),
            no_history: None,
            method: String::new(),
            path_template: String::new(),
            vars: Some(serde_json::json!({"id": ".data.id"})),
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: op_path
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_string_lossy()
                .to_string(),
            vars_jq: String::new(),
            vars_template: template_path
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_string_lossy()
                .to_string(),
            allow_errors: false,
        };

        let ok = graphql_cleanup_step(&mut ctx, &response_json, &step, 3).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("varsTemplate render failed"));
    }

    #[test]
    fn suite_cleanup_graphql_step_allow_errors_requires_expect_jq() {
        let fixture = GraphqlSetupFixture::new();
        let run_dir = fixture.root.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let op_path = fixture.root.join("ops/cleanup.graphql");
        std::fs::create_dir_all(op_path.parent().unwrap()).unwrap();
        std::fs::write(&op_path, "query Q { ok }\n").unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: &fixture.root,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({});
        let step = SuiteCleanupStep {
            step_type: "graphql".to_string(),
            config_dir: String::new(),
            url: "https://override.example/graphql".to_string(),
            env: String::new(),
            no_history: None,
            method: String::new(),
            path_template: String::new(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: op_path
                .strip_prefix(&fixture.root)
                .unwrap()
                .to_string_lossy()
                .to_string(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: true,
        };

        let ok = graphql_cleanup_step(&mut ctx, &response_json, &step, 4).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("expect.jq missing"));
    }

    #[test]
    fn suite_cleanup_graphql_step_invalid_vars_template_is_logged() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let op_dir = repo.join("ops");
        std::fs::create_dir_all(&op_dir).unwrap();
        let op_path = op_dir.join("cleanup.graphql");
        std::fs::write(&op_path, "query Q { ok }\n").unwrap();

        let template_path = repo.join("templates/vars.json");
        std::fs::create_dir_all(template_path.parent().unwrap()).unwrap();
        std::fs::write(&template_path, r#"{"id": {{id}}"#).unwrap();

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: None,
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: None,
        };

        let response_json = serde_json::json!({"data": {"id": "123"}});
        let step = SuiteCleanupStep {
            step_type: "graphql".to_string(),
            config_dir: String::new(),
            url: "https://override.example/graphql".to_string(),
            env: String::new(),
            no_history: None,
            method: String::new(),
            path_template: String::new(),
            vars: Some(serde_json::json!({"id": ".data.id"})),
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: op_path.to_string_lossy().to_string(),
            vars_jq: String::new(),
            vars_template: template_path.to_string_lossy().to_string(),
            allow_errors: false,
        };

        let err = graphql_cleanup_step(&mut ctx, &response_json, &step, 2).unwrap_err();
        assert!(err
            .to_string()
            .contains("varsTemplate rendered invalid JSON"));
    }

    #[test]
    fn suite_cleanup_run_case_missing_response_file_is_error() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let cleanup = SuiteCleanup::One(Box::new(SuiteCleanupStep {
            step_type: "rest".to_string(),
            config_dir: String::new(),
            url: String::new(),
            env: String::new(),
            no_history: None,
            method: "DELETE".to_string(),
            path_template: "/x".to_string(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: false,
        }));

        let defaults = SuiteDefaults::default();
        let missing_response = run_dir.join("missing.json");
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: Some(&missing_response),
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: Some(&cleanup),
        };

        let ok = run_case_cleanup(&mut ctx).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("missing main response file"));
    }

    #[test]
    fn suite_cleanup_run_case_unknown_step_type_is_error() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let run_dir = repo.join("out");
        std::fs::create_dir_all(&run_dir).unwrap();
        let stderr_file = run_dir.join("case.stderr.log");
        write_file(&stderr_file, b"").unwrap();

        let response_file = run_dir.join("response.json");
        std::fs::write(&response_file, br#"{"ok":true}"#).unwrap();

        let cleanup = SuiteCleanup::One(Box::new(SuiteCleanupStep {
            step_type: "mystery".to_string(),
            config_dir: String::new(),
            url: String::new(),
            env: String::new(),
            no_history: None,
            method: "DELETE".to_string(),
            path_template: "/x".to_string(),
            vars: None,
            token: String::new(),
            expect: None,
            expect_status: None,
            expect_jq: String::new(),
            jwt: String::new(),
            op: String::new(),
            vars_jq: String::new(),
            vars_template: String::new(),
            allow_errors: false,
        }));

        let defaults = SuiteDefaults::default();
        let mut ctx = CleanupContext {
            repo_root: repo,
            run_dir: &run_dir,
            case_id: "c",
            safe_id: "c",
            main_response_file: Some(&response_file),
            main_stderr_file: &stderr_file,
            allow_writes_flag: true,
            effective_env: "staging",
            effective_no_history: true,
            suite_defaults: &defaults,
            env_rest_url: "",
            env_gql_url: "",
            rest_config_dir: "setup/rest",
            rest_url: "",
            rest_token: "",
            gql_config_dir: "setup/graphql",
            gql_url: "",
            gql_jwt: "",
            access_token_for_case: "",
            auth_manager: None,
            cleanup: Some(&cleanup),
        };

        let ok = run_case_cleanup(&mut ctx).unwrap();
        assert!(!ok);
        let content = std::fs::read_to_string(&stderr_file).unwrap();
        assert!(content.contains("unknown step type"));
    }
}
