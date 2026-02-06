mod common;

use std::path::Path;

use agent_docs::config::{load_scope_config, CONFIG_FILE_NAME};
use agent_docs::model::{Context, Scope};

#[test]
fn add_full_flow_for_home_and_project_scopes() {
    let workspace = common::FixtureWorkspace::from_fixtures();
    common::write_text(
        &workspace.codex_home.join("TASK_TOOLS_EXTRA.md"),
        "# Fixture: home TASK_TOOLS_EXTRA\n",
    );
    common::write_text(
        &workspace.project_path.join("BINARY_DEPENDENCIES.md"),
        "# Fixture: project BINARY_DEPENDENCIES\n- tree\n- file\n",
    );

    let home_add = common::run_agent_docs_command(
        &workspace,
        &[
            "add",
            "--target",
            "home",
            "--context",
            "task-tools",
            "--scope",
            "home",
            "--path",
            "TASK_TOOLS_EXTRA.md",
            "--required",
            "--notes",
            "home task-tools extension",
        ],
    );
    assert!(
        home_add.success(),
        "add(home) should succeed, got code={} stderr={}",
        home_add.exit_code,
        home_add.stderr
    );
    assert!(
        home_add.stdout.contains("add: target=home action=inserted"),
        "expected add(home) stub output, got:\n{}",
        home_add.stdout
    );
    assert!(
        home_add.stdout.contains(&format!(
            "config={}",
            workspace.codex_home.join(CONFIG_FILE_NAME).display()
        )),
        "add(home) output should include config path, got:\n{}",
        home_add.stdout
    );

    let project_add = common::run_agent_docs_command(
        &workspace,
        &[
            "add",
            "--target",
            "project",
            "--context",
            "project-dev",
            "--scope",
            "project",
            "--path",
            "BINARY_DEPENDENCIES.md",
            "--required",
            "--notes",
            "project binary dependencies extension",
        ],
    );
    assert!(
        project_add.success(),
        "add(project) should succeed, got code={} stderr={}",
        project_add.exit_code,
        project_add.stderr
    );
    assert!(
        project_add
            .stdout
            .contains("add: target=project action=inserted"),
        "expected add(project) stub output, got:\n{}",
        project_add.stdout
    );
    assert!(
        project_add.stdout.contains(&format!(
            "config={}",
            workspace.project_path.join(CONFIG_FILE_NAME).display()
        )),
        "add(project) output should include config path, got:\n{}",
        project_add.stdout
    );

    let home_loaded = load_scope_config(Scope::Home, &workspace.codex_home)
        .expect("load home config")
        .expect("home config should exist");
    let home_entry = home_loaded
        .documents
        .iter()
        .find(|entry| entry.path == Path::new("TASK_TOOLS_EXTRA.md"))
        .expect("home extension entry should exist");
    assert_eq!(home_entry.context, Context::TaskTools);
    assert_eq!(home_entry.scope, Scope::Home);
    assert!(home_entry.required);
    assert_eq!(
        home_entry.notes.as_deref(),
        Some("home task-tools extension")
    );

    let project_loaded = load_scope_config(Scope::Project, &workspace.project_path)
        .expect("load project config")
        .expect("project config should exist");
    let project_entry = project_loaded
        .documents
        .iter()
        .find(|entry| entry.path == Path::new("BINARY_DEPENDENCIES.md"))
        .expect("project extension entry should exist");
    assert_eq!(project_entry.context, Context::ProjectDev);
    assert_eq!(project_entry.scope, Scope::Project);
    assert!(project_entry.required);
    assert_eq!(
        project_entry.notes.as_deref(),
        Some("project binary dependencies extension")
    );

    let task_tools_resolve = common::run_agent_docs_command(
        &workspace,
        &["resolve", "--context", "task-tools", "--format", "text"],
    );
    assert!(
        task_tools_resolve.success(),
        "resolve(task-tools) should succeed, got code={} stderr={}",
        task_tools_resolve.exit_code,
        task_tools_resolve.stderr
    );
    let task_tools_required = common::required_lines(&task_tools_resolve.stdout);
    assert_eq!(
        task_tools_required.len(),
        2,
        "task-tools should include builtin + extension required docs:\n{}",
        task_tools_resolve.stdout
    );
    assert!(
        task_tools_required
            .iter()
            .any(|line| line.contains("CLI_TOOLS.md") && line.contains("source=builtin")),
        "task-tools output should include builtin CLI_TOOLS.md:\n{}",
        task_tools_resolve.stdout
    );
    assert!(
        task_tools_required.iter().any(|line| {
            line.contains("TASK_TOOLS_EXTRA.md")
                && line.contains("source=extension-home")
                && line.contains("status=present")
        }),
        "task-tools output should include extension-home doc:\n{}",
        task_tools_resolve.stdout
    );

    let project_dev_resolve = common::run_agent_docs_command(
        &workspace,
        &["resolve", "--context", "project-dev", "--format", "text"],
    );
    assert!(
        project_dev_resolve.success(),
        "resolve(project-dev) should succeed, got code={} stderr={}",
        project_dev_resolve.exit_code,
        project_dev_resolve.stderr
    );
    let project_dev_required = common::required_lines(&project_dev_resolve.stdout);
    assert_eq!(
        project_dev_required.len(),
        2,
        "project-dev should include builtin + extension required docs:\n{}",
        project_dev_resolve.stdout
    );
    assert!(
        project_dev_required
            .iter()
            .any(|line| line.contains("DEVELOPMENT.md") && line.contains("source=builtin")),
        "project-dev output should include builtin DEVELOPMENT.md:\n{}",
        project_dev_resolve.stdout
    );
    assert!(
        project_dev_required.iter().any(|line| {
            line.contains("BINARY_DEPENDENCIES.md")
                && line.contains("source=extension-project")
                && line.contains("status=present")
        }),
        "project-dev output should include extension-project BINARY_DEPENDENCIES.md:\n{}",
        project_dev_resolve.stdout
    );
}
