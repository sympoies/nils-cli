use std::io::Write;
use std::path::Path;

use anyhow::Context;

use crate::suite::safety::writes_enabled;
use crate::suite::schema::SuiteCleanupStep;
use crate::Result;

mod context;
mod graphql;
mod rest;
mod template;

pub use context::CleanupContext;

fn append_log(path: &Path, line: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open log for append: {}", path.display()))?;
    writeln!(f, "{line}").context("append log line")?;
    Ok(())
}

fn append_log_from_text(path: &Path, text: &str) -> Result<()> {
    for line in text.lines() {
        append_log(path, line)?;
    }
    Ok(())
}

fn log_failure_with_stderr_file(path: &Path, header_line: &str, stderr_file: &Path) -> Result<()> {
    append_log(path, header_line)?;
    let stderr_text = std::fs::read_to_string(stderr_file).unwrap_or_default();
    append_log_from_text(path, &stderr_text)?;
    Ok(())
}

fn log_failure_with_error(path: &Path, header_line: &str, err: &anyhow::Error) -> Result<()> {
    append_log(path, header_line)?;
    append_log(path, &format!("{err:#}"))?;
    Ok(())
}

fn read_json_file(path: &Path) -> Result<serde_json::Value> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read JSON file: {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse JSON: {}", path.display()))?;
    Ok(v)
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
    rest::rest_cleanup_step(ctx, response_json, step, step_index)
}

fn graphql_cleanup_step(
    ctx: &mut CleanupContext<'_>,
    response_json: &serde_json::Value,
    step: &SuiteCleanupStep,
    step_index: usize,
) -> Result<bool> {
    graphql::graphql_cleanup_step(ctx, response_json, step, step_index)
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
    use super::template::{parse_vars_map, render_template};
    use super::*;
    use crate::suite::resolve::write_file;
    use crate::suite::runtime;
    use crate::suite::schema::{SuiteCleanup, SuiteCleanupStep, SuiteDefaults};
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
            runtime::resolve_rest_base_url(
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
            runtime::resolve_rest_base_url(repo_root, "setup/rest", "", "staging", &defaults2, "")
                .unwrap(),
            "https://defaults.example"
        );

        assert_eq!(
            runtime::resolve_rest_base_url(
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
            runtime::resolve_rest_base_url(repo_root, "setup/rest", "", "staging", &defaults, "")
                .unwrap(),
            "https://fromfile.example"
        );
    }

    #[test]
    fn suite_cleanup_gql_url_resolution_precedence() {
        let tmp = TempDir::new().unwrap();
        let repo_root = tmp.path();
        let defaults = SuiteDefaults::default();

        assert_eq!(
            runtime::resolve_gql_url(
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
            runtime::resolve_gql_url(repo_root, "setup/graphql", "", "staging", &defaults2, "")
                .unwrap(),
            "https://defaults.example/graphql"
        );

        assert_eq!(
            runtime::resolve_gql_url(
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
            runtime::resolve_gql_url(repo_root, "setup/graphql", "", "staging", &defaults, "")
                .unwrap(),
            "https://fromfile.example/graphql"
        );
    }

    #[test]
    fn suite_cleanup_rest_url_selection_uses_rest_endpoints_env() {
        let fixture = RestSetupFixture::new();
        fixture.write_endpoints_env("REST_URL_STAGING=https://fromfile.example\n");
        let defaults = SuiteDefaults::default();

        let url = runtime::resolve_rest_base_url(
            &fixture.root,
            "setup/rest",
            "",
            "staging",
            &defaults,
            "",
        )
        .unwrap();

        assert_eq!(url, "https://fromfile.example");
    }

    #[test]
    fn suite_cleanup_graphql_url_selection_uses_graphql_endpoints_env() {
        let fixture = GraphqlSetupFixture::new();
        fixture.write_endpoints_env("GQL_URL_STAGING=https://fromfile.example/graphql\n");
        let defaults = SuiteDefaults::default();

        let url =
            runtime::resolve_gql_url(&fixture.root, "setup/graphql", "", "staging", &defaults, "")
                .unwrap();

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
