use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use agent_out::{CleanupItem, CleanupPlan, CleanupSummary};
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const OLD_UNIX: u64 = 1_577_836_800; // 2020-01-01T00:00:00Z

fn run(dir: &Path, args: &[&str]) -> CmdOutput {
    run_resolved("agent-out", args, &CmdOptions::new().with_cwd(dir))
}

/// Set every entry's mtime to 2020-01-01, children before parents.
fn age_tree(path: &Path) {
    let metadata = fs::symlink_metadata(path).expect("metadata");
    if metadata.is_dir() {
        for entry in fs::read_dir(path).expect("read dir") {
            age_tree(&entry.expect("entry").path());
        }
    }
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(OLD_UNIX);
    fs::File::open(path)
        .expect("open for times")
        .set_modified(time)
        .expect("set mtime");
}

fn make_run(projects: &Path, repo: &str, run: &str) -> PathBuf {
    let dir = projects.join(repo).join(run);
    fs::create_dir_all(dir.join("logs")).expect("run dir");
    fs::write(dir.join("logs/output.txt"), "output").expect("run file");
    dir
}

fn today_run_id() -> String {
    chrono::Local::now()
        .format("%Y%m%d-%H%M%S-today")
        .to_string()
}

struct Fixture {
    tmp: tempfile::TempDir,
    agent_home: PathBuf,
    projects: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let agent_home = tmp.path().join("agent-home");
        let projects = agent_home.join("out/projects");
        fs::create_dir_all(&projects).expect("projects");
        Self {
            tmp,
            agent_home,
            projects,
        }
    }

    fn home_arg(&self) -> String {
        self.agent_home.to_string_lossy().to_string()
    }

    fn plan(&self, extra: &[&str]) -> CmdOutput {
        let home = self.home_arg();
        let mut args = vec![
            "cleanup",
            "plan",
            "--agent-home",
            home.as_str(),
            "--include-projects",
            "--format",
            "json",
        ];
        args.extend_from_slice(extra);
        run(self.tmp.path(), &args)
    }

    fn apply(&self, plan: &Value) -> CmdOutput {
        let plan_file = self.tmp.path().join("plan.json");
        fs::write(&plan_file, serde_json::to_vec(plan).expect("plan json")).expect("plan file");
        let digest = plan["result"]["plan_digest"]
            .as_str()
            .expect("digest")
            .to_string();
        let plan_arg = plan_file.to_string_lossy().to_string();
        let home = self.home_arg();
        run(
            self.tmp.path(),
            &[
                "cleanup",
                "apply",
                "--plan-file",
                &plan_arg,
                "--confirm-digest",
                &digest,
                "--agent-home",
                &home,
                "--format",
                "json",
            ],
        )
    }
}

fn item<'a>(plan: &'a Value, run: &str) -> &'a Value {
    plan["result"]["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["name"] == run)
        .unwrap_or_else(|| panic!("no plan row for {run}"))
}

/// Re-sign a plan the way apply recomputes it, using the crate's own types.
fn resign(envelope: &mut Value) {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        agent_home: &'a str,
        out_root: &'a str,
        out_root_exists: bool,
        include_projects: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        project_retention_days: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        project_retention_cutoff_unix: Option<i64>,
        items: &'a [CleanupItem],
        summary: &'a CleanupSummary,
    }
    let mut plan: CleanupPlan =
        serde_json::from_value(envelope["result"].clone()).expect("cleanup plan");
    let bytes = serde_json::to_vec(&DigestInput {
        agent_home: &plan.agent_home,
        out_root: &plan.out_root,
        out_root_exists: plan.out_root_exists,
        include_projects: plan.include_projects,
        project_retention_days: plan.project_retention_days,
        project_retention_cutoff_unix: plan.project_retention_cutoff_unix,
        items: &plan.items,
        summary: &plan.summary,
    })
    .expect("digest input");
    let hex = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    plan.plan_digest = format!("sha256:{hex}");
    envelope["result"] = serde_json::to_value(&plan).expect("plan value");
}

#[test]
fn retention_policy_marks_only_idle_run_id_directories_for_deletion() {
    let fixture = Fixture::new();
    let old = make_run(&fixture.projects, "owner__repo", "20200101-000000-old");
    let touched = make_run(&fixture.projects, "owner__repo", "20200101-000000-touched");
    let evidence = make_run(&fixture.projects, "owner__repo", "20200101-000000-evidence");
    fs::write(evidence.join("test-first-evidence.json"), "{}").expect("marker");
    let adhoc = make_run(&fixture.projects, "owner__repo", "adhoc-run");
    let day_only = make_run(&fixture.projects, "owner__repo", "20200101-dayonly");
    for dir in [&old, &touched, &evidence, &adhoc, &day_only] {
        age_tree(dir);
    }
    fs::write(touched.join("logs/new.txt"), "recent").expect("recent write");
    let today = today_run_id();
    make_run(&fixture.projects, "owner__repo", &today);

    let output = fixture.plan(&["--project-retention-days", "30"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let plan = output.stdout_json();
    assert_eq!(plan["result"]["project_retention_days"], 30);
    assert!(plan["result"]["project_retention_cutoff_unix"].is_i64());

    let old_row = item(&plan, "20200101-000000-old");
    assert_eq!(old_row["action"], "delete");
    assert_eq!(old_row["category"], "project-artifact");
    assert!(
        old_row["tree_identity"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_eq!(item(&plan, "20200101-dayonly")["action"], "delete");

    let touched_row = item(&plan, "20200101-000000-touched");
    assert_eq!(touched_row["action"], "needs-policy");
    assert!(
        touched_row["reason"]
            .as_str()
            .unwrap()
            .contains("changed within")
    );
    assert_eq!(
        item(&plan, "20200101-000000-evidence")["action"],
        "preserve"
    );
    let adhoc_row = item(&plan, "adhoc-run");
    assert_eq!(adhoc_row["action"], "needs-policy");
    assert!(adhoc_row["reason"].as_str().unwrap().contains("run id"));
    let today_row = item(&plan, &today);
    assert_eq!(today_row["action"], "needs-policy");
    assert_eq!(plan["result"]["summary"]["delete"], 2);
}

#[test]
fn plan_without_retention_flag_keeps_the_previous_contract() {
    let fixture = Fixture::new();
    let old = make_run(&fixture.projects, "owner__repo", "20200101-000000-old");
    age_tree(&old);

    let output = fixture.plan(&[]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let plan = output.stdout_json();
    assert!(plan["result"].get("project_retention_days").is_none());
    assert!(
        plan["result"]
            .get("project_retention_cutoff_unix")
            .is_none()
    );
    let row = item(&plan, "20200101-000000-old");
    assert_eq!(row["action"], "needs-policy");
    assert!(row.get("tree_identity").is_none());
    assert_eq!(plan["result"]["summary"]["delete"], 0);
}

#[cfg(unix)]
#[test]
fn unreadable_subtree_is_preserved_while_the_rest_is_planned() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let locked_run = make_run(&fixture.projects, "owner__repo", "20200101-000000-locked");
    let locked = locked_run.join("store/manager-state");
    fs::create_dir_all(&locked).expect("locked dir");
    fs::write(locked.join("state"), "secret").expect("locked file");
    let old = make_run(&fixture.projects, "owner__repo", "20200101-000000-old");
    age_tree(&locked_run);
    age_tree(&old);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod 000");
    if fs::read_dir(&locked).is_ok() {
        // Running as root: permissions cannot make the subtree unreadable.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");
        return;
    }

    let output = fixture.plan(&["--project-retention-days", "30"]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let plan = output.stdout_json();

    let locked_row = item(&plan, "20200101-000000-locked");
    assert_eq!(locked_row["category"], "unreadable");
    assert_eq!(locked_row["action"], "preserve");
    assert_eq!(locked_row["diagnostic"]["code"], "cleanup-read-failed");
    assert!(
        locked_row["diagnostic"]["path"]
            .as_str()
            .unwrap()
            .ends_with("manager-state")
    );
    assert_eq!(plan["result"]["summary"]["unreadable"], 1);
    assert_eq!(item(&plan, "20200101-000000-old")["action"], "delete");
}

#[test]
fn apply_deletes_idle_runs_and_skips_a_run_that_changed() {
    let fixture = Fixture::new();
    let idle = make_run(&fixture.projects, "owner__repo", "20200101-000000-idle");
    let drift = make_run(&fixture.projects, "owner__repo", "20200101-000000-drift");
    age_tree(&idle);
    age_tree(&drift);

    let plan = fixture
        .plan(&["--project-retention-days", "30"])
        .stdout_json();
    fs::write(drift.join("logs/output.txt"), "rewritten").expect("rewrite");
    age_tree(&drift);

    let output = fixture.apply(&plan);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let report = output.stdout_json();
    assert_eq!(report["result"]["summary"]["deleted"], 1);
    assert_eq!(report["result"]["summary"]["skipped"], 1);
    assert_eq!(report["result"]["summary"]["failed"], 0);
    assert!(!idle.exists());
    assert!(drift.join("logs/output.txt").exists());
}

#[test]
fn apply_rejects_a_forged_retention_window_before_deleting() {
    let fixture = Fixture::new();
    let idle = make_run(&fixture.projects, "owner__repo", "20200101-000000-idle");
    age_tree(&idle);
    let mut plan = fixture
        .plan(&["--project-retention-days", "30"])
        .stdout_json();
    // A cutoff in the future would admit runs the policy never allowed.
    plan["result"]["project_retention_cutoff_unix"] = Value::from(4_102_444_800_i64);
    resign(&mut plan);

    let output = fixture.apply(&plan);
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "cleanup-retention-policy-invalid"
    );
    assert!(idle.exists());
}

#[test]
fn apply_rejects_a_project_delete_without_a_retention_policy() {
    let fixture = Fixture::new();
    let idle = make_run(&fixture.projects, "owner__repo", "20200101-000000-idle");
    age_tree(&idle);
    let mut plan = fixture
        .plan(&["--project-retention-days", "30"])
        .stdout_json();
    let result = plan["result"].as_object_mut().expect("result");
    result.remove("project_retention_days");
    result.remove("project_retention_cutoff_unix");
    resign(&mut plan);

    let output = fixture.apply(&plan);
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "cleanup-delete-shape-invalid"
    );
    assert!(idle.exists());
}

#[test]
fn retention_flag_requires_include_projects_and_a_positive_day_count() {
    let fixture = Fixture::new();
    let home = fixture.home_arg();
    let without_projects = run(
        fixture.tmp.path(),
        &[
            "cleanup",
            "plan",
            "--agent-home",
            &home,
            "--project-retention-days",
            "30",
        ],
    );
    assert_ne!(without_projects.code, 0);
    let zero = fixture.plan(&["--project-retention-days", "0"]);
    assert_ne!(zero.code, 0);
}
