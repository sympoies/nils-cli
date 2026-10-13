use super::*;
use crate::backend::BackendSuccess;
use crate::provider::DetectionSource;
use pretty_assertions::assert_eq;
use std::cell::RefCell;
use std::collections::VecDeque;

struct Runner {
    responses: RefCell<VecDeque<String>>,
    calls: RefCell<Vec<Vec<String>>>,
}
impl Runner {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
}
impl BackendRunner for Runner {
    fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
        self.calls.borrow_mut().push(call.plan_argv());
        self.responses
            .borrow_mut()
            .pop_front()
            .map(|stdout| BackendSuccess {
                stdout,
                stderr: String::new(),
            })
            .ok_or_else(unavailable)
    }
}
fn ctx(provider: Provider) -> ProviderContext {
    ProviderContext {
        provider,
        host: if provider == Provider::GitLab {
            "gitlab.example.com"
        } else {
            "github.com"
        }
        .into(),
        repo: Some("example/project".into()),
        source: DetectionSource::Flag,
    }
}
fn row(gitlab: bool, number: u64) -> Value {
    if gitlab {
        serde_json::json!({"iid":number,"web_url":format!("https://gitlab.example.com/example/project/-/merge_requests/{number}"),"state":"opened","target_branch":"feat/parent"})
    } else {
        serde_json::json!({"number":number,"html_url":format!("https://github.com/example/project/pull/{number}"),"state":"open","base":{"ref":"feat/parent"}})
    }
}

#[test]
fn missing_source_identity_refuses_cleanup_before_provider_reads() {
    for provider in [Provider::GitHub, Provider::GitLab] {
        for overridden in [false, true] {
            let runner = Runner::new(vec![if provider == Provider::GitHub {
                "[[]]".into()
            } else {
                "[]".into()
            }]);
            assert_eq!(
                check(
                    &runner,
                    &ctx(provider),
                    "example/project",
                    None,
                    "feat/parent",
                    7,
                    overridden
                )
                .unwrap_err()
                .kind(),
                "branch_dependencies_unavailable"
            );
            assert!(runner.calls.borrow().is_empty());
        }
    }
}

#[test]
fn gitlab_reads_next_page_and_uses_source_project_identity() {
    let first = Value::Array((10..110).map(|number| row(true, number)).collect());
    let runner = Runner::new(vec![
        first.to_string(),
        serde_json::json!([row(true, 110)]).to_string(),
    ]);
    let cleanup = check(
        &runner,
        &ctx(Provider::GitLab),
        "example/project",
        Some("gitlab-project-id:42"),
        "feat/parent",
        7,
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(cleanup.dependents.len(), 101);
    let calls = runner.calls.borrow();
    assert!(calls[0].iter().any(|arg| arg.contains(
        "projects/42/merge_requests?state=opened&target_branch=feat%2Fparent&per_page=100&page=1"
    )));
    assert!(calls[1].iter().any(|arg| arg.ends_with("page=2")));
}

#[test]
fn gitlab_later_page_failure_refuses_cleanup() {
    let first = Value::Array((10..110).map(|number| row(true, number)).collect());
    let runner = Runner::new(vec![first.to_string()]);
    assert_eq!(
        check(
            &runner,
            &ctx(Provider::GitLab),
            "example/project",
            Some("example/project"),
            "feat/parent",
            7,
            false
        )
        .unwrap_err()
        .kind(),
        "branch_dependencies_unavailable"
    );
}

#[test]
fn github_uses_source_repository_and_encoded_branch() {
    let runner = Runner::new(vec!["[[]]".into()]);
    assert!(
        check(
            &runner,
            &ctx(Provider::GitHub),
            "example/project",
            Some("example/fork"),
            "feat/parent",
            7,
            false
        )
        .unwrap()
        .is_none()
    );
    let calls = runner.calls.borrow();
    assert!(calls[0].iter().any(|arg| arg == "--paginate"));
    assert!(calls[0].iter().any(|arg| arg == "--slurp"));
    assert!(calls[0].iter().any(|arg| arg == "repos/example/fork/pulls?state=open&base=feat%2Fparent&per_page=100"));
}

#[test]
fn dependency_backend_failure_remains_fail_closed_with_override() {
    for provider in [Provider::GitHub, Provider::GitLab] {
        let runner = Runner::new(Vec::new());
        assert_eq!(
            check(
                &runner,
                &ctx(provider),
                "example/project",
                Some("example/project"),
                "feat/parent",
                7,
                true
            )
            .unwrap_err()
            .kind(),
            "branch_dependencies_unavailable"
        );
    }
}

#[test]
fn github_unknown_automatic_deletion_setting_refuses_merge() {
    let runner = Runner::new(vec![
        serde_json::json!([[row(false, 8)]]).to_string(),
        "{}".into(),
    ]);
    assert_eq!(
        check(
            &runner,
            &ctx(Provider::GitHub),
            "example/project",
            Some("example/project"),
            "feat/parent",
            7,
            false
        )
        .unwrap_err()
        .kind(),
        "branch_dependencies_unavailable"
    );
}

#[test]
fn fork_local_number_collision_remains_a_dependency() {
    for provider in [Provider::GitHub, Provider::GitLab] {
        let gitlab = provider == Provider::GitLab;
        let rows = serde_json::json!([row(gitlab, 7)]);
        let response = if gitlab {
            rows.to_string()
        } else {
            serde_json::json!([rows]).to_string()
        };
        let runner = Runner::new(vec![response]);
        let cleanup = check(
            &runner,
            &ctx(provider),
            "example/project",
            Some(if gitlab {
                "gitlab-project-id:42"
            } else {
                "example/fork"
            }),
            "feat/parent",
            7,
            false,
        )
        .unwrap()
        .expect("fork-local dependent must not be discarded");
        assert_eq!(cleanup.dependents[0].number, 7);
        assert!(!cleanup.overridden);
    }
}

#[test]
fn github_enterprise_reads_use_the_selected_host() {
    let mut context = ctx(Provider::GitHub);
    context.host = "github.example.com".into();
    let runner = Runner::new(vec![
        serde_json::json!([[row(false, 8)]]).to_string(),
        r#"{"delete_branch_on_merge":false}"#.into(),
    ]);
    check(
        &runner,
        &context,
        "example/project",
        Some("example/project"),
        "feat/parent",
        7,
        false,
    )
    .unwrap();
    for call in runner.calls.borrow().iter() {
        assert!(
            call.iter().any(|arg| arg == "github.example.com"),
            "{call:?}"
        );
    }
}
