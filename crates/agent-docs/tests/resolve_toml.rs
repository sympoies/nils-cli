mod common;

use agent_docs::config::CONFIG_FILE_NAME;

#[test]
fn resolve_toml_malformed_config_returns_non_zero_with_actionable_stderr() {
    let workspace = common::FixtureWorkspace::from_fixtures();
    common::write_text(
        &workspace.project_path.join(CONFIG_FILE_NAME),
        r#"
[[document
context = "project-dev"
scope = "project"
path = "BINARY_DEPENDENCIES.md"
required = true
when = "always"
"#,
    );

    let output = common::run_agent_docs_command(
        &workspace,
        &["resolve", "--context", "project-dev", "--format", "text"],
    );

    assert_eq!(
        output.exit_code, 3,
        "malformed TOML should fail with config exit code, stderr:\n{}",
        output.stderr
    );
    assert!(
        output.stdout.is_empty(),
        "resolve should not emit stdout on parse failure, got:\n{}",
        output.stdout
    );
    assert!(
        output.stderr.contains("error:"),
        "stderr should include error prefix, got:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains(
            &workspace
                .project_path
                .join(CONFIG_FILE_NAME)
                .display()
                .to_string()
        ),
        "stderr should include config path, got:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("[parse]"),
        "stderr should include parse classification, got:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("invalid TOML in AGENT_DOCS.toml"),
        "stderr should include actionable parse message, got:\n{}",
        output.stderr
    );
}

#[test]
fn resolve_toml_project_dev_includes_binary_dependencies_from_extension() {
    let workspace = common::FixtureWorkspace::from_fixtures();
    common::write_text(
        &workspace.project_path.join(CONFIG_FILE_NAME),
        r#"
[[document]]
context = "project-dev"
scope = "project"
path = "BINARY_DEPENDENCIES.md"
required = true
when = "always"
notes = "project binary dependencies from toml"
"#,
    );

    let output = common::run_agent_docs_command(
        &workspace,
        &["resolve", "--context", "project-dev", "--format", "text"],
    );
    assert!(
        output.success(),
        "resolve(project-dev) should succeed, got code={} stderr={}",
        output.exit_code,
        output.stderr
    );

    let required_lines = common::required_lines(&output.stdout);
    assert_eq!(
        required_lines.len(),
        2,
        "project-dev should include builtin + extension required docs:\n{}",
        output.stdout
    );
    assert!(
        required_lines
            .iter()
            .any(|line| line.contains("DEVELOPMENT.md") && line.contains("source=builtin")),
        "project-dev output should include builtin DEVELOPMENT.md:\n{}",
        output.stdout
    );

    let binary_line = required_lines
        .iter()
        .find(|line| line.contains("BINARY_DEPENDENCIES.md"))
        .expect("project-dev output should include BINARY_DEPENDENCIES.md extension");
    assert!(
        binary_line.contains("source=extension-project"),
        "extension line should be sourced from project TOML: {binary_line}"
    );
    assert!(
        binary_line.contains("status=present"),
        "extension line should report present status: {binary_line}"
    );
    assert!(
        binary_line.contains("project binary dependencies from toml"),
        "extension line should include notes-driven why message: {binary_line}"
    );
}
