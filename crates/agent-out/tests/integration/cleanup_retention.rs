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
        #[serde(skip_serializing_if = "Option::is_none")]
        scratch_retention_days: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scratch_retention_cutoff_unix: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        build_retention_days: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        build_retention_cutoff_unix: Option<i64>,
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
        scratch_retention_days: plan.scratch_retention_days,
        scratch_retention_cutoff_unix: plan.scratch_retention_cutoff_unix,
        build_retention_days: plan.build_retention_days,
        build_retention_cutoff_unix: plan.build_retention_cutoff_unix,
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

/// True when permission bits cannot restrict this process (for example root).
#[cfg(unix)]
fn permissions_are_bypassed(probe: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(probe).expect("probe dir");
    fs::set_permissions(probe, fs::Permissions::from_mode(0o000)).expect("chmod probe");
    let bypassed = fs::read_dir(probe).is_ok();
    fs::set_permissions(probe, fs::Permissions::from_mode(0o755)).expect("restore probe");
    bypassed
}

#[cfg(unix)]
#[test]
fn a_failed_project_run_delete_is_recorded_and_other_rows_continue() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    if permissions_are_bypassed(&fixture.tmp.path().join("probe")) {
        return;
    }
    let first = make_run(&fixture.projects, "owner__repo", "20200101-000000-first");
    let stuck = make_run(&fixture.projects, "owner__repo", "20200101-000000-stuck");
    // A read-only directory keeps its files from being removed.
    fs::set_permissions(stuck.join("logs"), fs::Permissions::from_mode(0o555)).expect("chmod");
    age_tree(&first);
    age_tree(&stuck);

    let plan = fixture
        .plan(&["--project-retention-days", "30"])
        .stdout_json();
    assert_eq!(plan["result"]["summary"]["delete"], 2);
    let output = fixture.apply(&plan);
    fs::set_permissions(stuck.join("logs"), fs::Permissions::from_mode(0o755)).expect("restore");

    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let report = output.stdout_json();
    assert_eq!(report["result"]["summary"]["deleted"], 1);
    assert_eq!(report["result"]["summary"]["failed"], 1);
    let failed = report["result"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["status"] == "failed")
        .expect("failed entry");
    assert!(
        failed["path"]
            .as_str()
            .unwrap()
            .ends_with("20200101-000000-stuck")
    );
    assert!(!first.exists());
    assert!(stuck.join("logs/output.txt").exists());
}

#[cfg(unix)]
#[test]
fn symlinked_repo_directories_never_yield_project_run_deletes() {
    let fixture = Fixture::new();
    let real = make_run(&fixture.projects, "owner__repo", "20200101-000000-real");
    let elsewhere = fixture.agent_home.join("out/playwright");
    let hidden = make_run(&elsewhere, "tool", "20200101-000000-hidden");
    age_tree(&real);
    age_tree(&hidden);
    std::os::unix::fs::symlink(elsewhere.join("tool"), fixture.projects.join("linked"))
        .expect("symlink repo dir");

    let mut plan = fixture
        .plan(&["--project-retention-days", "30"])
        .stdout_json();
    let items = plan["result"]["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .all(|item| !item["path"].as_str().unwrap().contains("/linked/")),
        "the planner must not descend into a symlinked repo directory"
    );

    // A forged row that reaches the hidden run through the link is refused.
    let mut forged = item(&plan, "20200101-000000-real").clone();
    forged["name"] = Value::from("20200101-000000-hidden");
    // Build from the plan's own out_root: on macOS it is the canonical
    // /private/var form, and the forged row must stay lexically inside it so
    // the symlink check (not the out-of-root check) is what refuses it.
    let out_root = PathBuf::from(plan["result"]["out_root"].as_str().expect("out_root"));
    forged["path"] = Value::from(
        out_root
            .join("projects/linked/20200101-000000-hidden")
            .to_string_lossy()
            .to_string(),
    );
    plan["result"]["items"]
        .as_array_mut()
        .expect("items")
        .push(forged);
    resign(&mut plan);
    let output = fixture.apply(&plan);
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "cleanup-delete-shape-invalid"
    );
    assert!(hidden.exists());
    assert!(real.exists());
}

#[cfg(unix)]
#[test]
fn unreadable_top_level_and_repo_entries_become_preserved_rows() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    if permissions_are_bypassed(&fixture.tmp.path().join("probe")) {
        return;
    }
    let loose = fixture.agent_home.join("out/loose-report");
    fs::create_dir_all(loose.join("inner")).expect("loose dir");
    let repo = fixture.projects.join("locked__repo");
    make_run(&fixture.projects, "locked__repo", "20200101-000000-run");
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o000)).expect("chmod loose");
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o000)).expect("chmod repo");

    let output = fixture.plan(&["--project-retention-days", "30"]);
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o755)).expect("restore loose");
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o755)).expect("restore repo");

    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let plan = output.stdout_json();
    for name in ["loose-report", "locked__repo"] {
        let row = item(&plan, name);
        assert_eq!(row["category"], "unreadable", "row {name}");
        assert_eq!(row["action"], "preserve", "row {name}");
    }
    assert_eq!(plan["result"]["summary"]["unreadable"], 2);
}

#[test]
fn plans_without_unreadable_rows_omit_the_unreadable_count() {
    let fixture = Fixture::new();
    make_run(&fixture.projects, "owner__repo", "20200101-000000-run");
    let plan = fixture.plan(&[]).stdout_json();
    assert!(plan["result"]["summary"].get("unreadable").is_none());
}

/// Write-location classes: evidence and scratch runs live at distinct roots,
/// run-scoped builds live outside `out/`, and cleanup plans apply per-class
/// retention and flag build trees found inside evidence runs.
///
/// Inline here for the same reason as `cli::classes`: new files under this
/// crate directory are refused by the artifact-routing hook.
mod write_classes {
    use std::fs;
    use std::path::{Path, PathBuf};

    use nils_test_support::cmd::CmdOutput;
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use super::{age_tree, run, today_run_id};

    const OLD_RUN: &str = "20200101-000000-old";

    struct Fixture {
        tmp: tempfile::TempDir,
        home: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let home = tmp.path().join("agent-home");
            fs::create_dir_all(&home).expect("agent home");
            Self { tmp, home }
        }

        fn home_arg(&self) -> String {
            self.home.to_string_lossy().to_string()
        }

        fn make_run(&self, class_root: &str, repo: Option<&str>, name: &str) -> PathBuf {
            let mut dir = self.home.join(class_root);
            if let Some(repo) = repo {
                dir = dir.join(repo);
            }
            let dir = dir.join(name);
            fs::create_dir_all(dir.join("logs")).expect("run dir");
            fs::write(dir.join("logs/output.txt"), "output").expect("run file");
            dir
        }

        fn plan(&self, extra: &[&str]) -> CmdOutput {
            let home = self.home_arg();
            let mut args = vec![
                "cleanup",
                "plan",
                "--agent-home",
                home.as_str(),
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

    fn row<'a>(plan: &'a Value, name: &str) -> &'a Value {
        plan["result"]["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|item| item["name"] == name)
            .unwrap_or_else(|| panic!("no plan row for {name}"))
    }

    /// Re-sign a hand-edited plan over the digest input apply recomputes.
    fn resign(envelope: &mut Value) {
        #[derive(Serialize)]
        struct DigestInput<'a> {
            agent_home: &'a str,
            out_root: &'a str,
            out_root_exists: bool,
            include_projects: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            project_retention_days: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            project_retention_cutoff_unix: Option<i64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            scratch_retention_days: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            scratch_retention_cutoff_unix: Option<i64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            build_retention_days: Option<u64>,
            #[serde(skip_serializing_if = "Option::is_none")]
            build_retention_cutoff_unix: Option<i64>,
            items: &'a Value,
            summary: &'a Value,
        }
        let plan = &envelope["result"];
        let bytes = serde_json::to_vec(&DigestInput {
            agent_home: plan["agent_home"].as_str().unwrap(),
            out_root: plan["out_root"].as_str().unwrap(),
            out_root_exists: plan["out_root_exists"].as_bool().unwrap(),
            include_projects: plan["include_projects"].as_bool().unwrap(),
            project_retention_days: plan["project_retention_days"].as_u64(),
            project_retention_cutoff_unix: plan["project_retention_cutoff_unix"].as_i64(),
            scratch_retention_days: plan["scratch_retention_days"].as_u64(),
            scratch_retention_cutoff_unix: plan["scratch_retention_cutoff_unix"].as_i64(),
            build_retention_days: plan["build_retention_days"].as_u64(),
            build_retention_cutoff_unix: plan["build_retention_cutoff_unix"].as_i64(),
            items: &plan["items"],
            summary: &plan["summary"],
        })
        .expect("digest input");
        let hex = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        envelope["result"]["plan_digest"] = Value::String(format!("sha256:{hex}"));
    }

    #[test]
    fn project_allocates_evidence_by_default_and_scratch_at_a_distinct_root() {
        let fixture = Fixture::new();
        let home = fixture.home_arg();
        let base = [
            "project",
            "--topic",
            "unpack",
            "--repo-slug",
            "owner/repo",
            "--agent-home",
            home.as_str(),
            "--format",
            "json",
        ];

        let evidence = run(fixture.tmp.path(), &base);
        assert_eq!(evidence.code, 0, "stderr={}", evidence.stderr_text());
        let evidence = evidence.stdout_json();
        assert_eq!(evidence["result"]["kind"], "evidence");
        let evidence_path = evidence["result"]["path"].as_str().unwrap();
        assert!(
            evidence_path.contains("/out/projects/owner__repo/"),
            "{evidence_path}"
        );

        let mut scratch_args = base.to_vec();
        scratch_args.extend(["--kind", "scratch", "--mkdir"]);
        let scratch = run(fixture.tmp.path(), &scratch_args);
        assert_eq!(scratch.code, 0, "stderr={}", scratch.stderr_text());
        let scratch = scratch.stdout_json();
        assert_eq!(scratch["result"]["kind"], "scratch");
        let scratch_path = scratch["result"]["path"].as_str().unwrap();
        assert!(
            scratch_path.contains("/out/scratch/owner__repo/"),
            "{scratch_path}"
        );
        assert!(scratch_path.ends_with("-unpack"), "{scratch_path}");
        assert!(Path::new(scratch_path).is_dir());
        assert!(!fixture.home.join("out/projects").exists());
    }

    #[test]
    fn project_kind_is_exported_in_env_format_and_rejects_unknown_values() {
        let fixture = Fixture::new();
        let home = fixture.home_arg();
        let env = run(
            fixture.tmp.path(),
            &[
                "project",
                "--topic",
                "unpack",
                "--repo-slug",
                "owner/repo",
                "--agent-home",
                &home,
                "--kind",
                "scratch",
                "--format",
                "env",
            ],
        );
        assert_eq!(env.code, 0, "stderr={}", env.stderr_text());
        assert!(
            env.stdout_text().contains("AGENT_OUT_KIND='scratch'"),
            "{}",
            env.stdout_text()
        );

        let bad = run(
            fixture.tmp.path(),
            &[
                "project",
                "--topic",
                "unpack",
                "--agent-home",
                &home,
                "--kind",
                "build",
            ],
        );
        assert_eq!(bad.code, 64);
    }

    #[test]
    fn build_allocates_a_run_scoped_directory_outside_out() {
        let fixture = Fixture::new();
        let home = fixture.home_arg();
        let output = run(
            fixture.tmp.path(),
            &[
                "build",
                "--topic",
                "release check",
                "--agent-home",
                &home,
                "--mkdir",
                "--format",
                "json",
            ],
        );
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        let json = output.stdout_json();
        assert_eq!(json["schema_version"], "cli.agent-out.build.v1");
        let path = json["result"]["path"].as_str().unwrap();
        let run_id = json["result"]["run_id"].as_str().unwrap();
        assert!(run_id.ends_with("-release-check"), "{run_id}");
        assert!(path.ends_with(&format!("/build/run/{run_id}")), "{path}");
        assert!(Path::new(path).is_dir());
        assert!(
            !fixture.home.join("out").exists(),
            "build must not use out/"
        );

        let env = run(
            fixture.tmp.path(),
            &[
                "build",
                "--topic",
                "release check",
                "--agent-home",
                &home,
                "--format",
                "env",
            ],
        );
        assert_eq!(env.code, 0, "stderr={}", env.stderr_text());
        let stdout = env.stdout_text();
        assert!(stdout.contains("CARGO_TARGET_DIR='"), "{stdout}");
        assert!(stdout.contains("AGENT_OUT_KIND='build'"), "{stdout}");
    }

    #[test]
    fn audit_accepts_the_scratch_root() {
        let fixture = Fixture::new();
        fixture.make_run("out/scratch", Some("owner__repo"), OLD_RUN);
        let home = fixture.home_arg();
        let output = run(
            fixture.tmp.path(),
            &["audit", "--agent-home", &home, "--strict"],
        );
        assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    }

    #[test]
    fn plan_applies_short_scratch_retention_by_default_and_shows_every_class() {
        let fixture = Fixture::new();
        let old = fixture.make_run("out/scratch", Some("owner__repo"), OLD_RUN);
        age_tree(&old);
        let today = today_run_id();
        fixture.make_run("out/scratch", Some("owner__repo"), &today);

        let output = fixture.plan(&[]);
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        let plan = output.stdout_json();
        assert_eq!(plan["result"]["scratch_retention_days"], 3);
        assert_eq!(plan["result"]["build_retention_days"], 3);
        assert!(plan["result"]["scratch_retention_cutoff_unix"].is_i64());
        assert!(plan["result"].get("project_retention_days").is_none());

        let old_row = row(&plan, OLD_RUN);
        assert_eq!(old_row["category"], "scratch-artifact");
        assert_eq!(old_row["action"], "delete");
        assert!(
            old_row["tree_identity"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        let today_row = row(&plan, &today);
        assert_eq!(today_row["action"], "preserve");
        assert!(
            today_row["reason"]
                .as_str()
                .unwrap()
                .contains("3-day retention window")
        );

        let home = fixture.home_arg();
        let text = run(
            fixture.tmp.path(),
            &[
                "cleanup",
                "plan",
                "--agent-home",
                &home,
                "--scratch-retention-days",
                "7",
            ],
        );
        assert_eq!(text.code, 0, "stderr={}", text.stderr_text());
        let stdout = text.stdout_text();
        assert!(stdout.contains("evidence retention: unset"), "{stdout}");
        assert!(stdout.contains("scratch retention: 7 days"), "{stdout}");
        assert!(stdout.contains("build retention: 3 days"), "{stdout}");
    }

    #[test]
    fn plan_rejects_a_zero_class_retention() {
        let fixture = Fixture::new();
        for flag in ["--scratch-retention-days", "--build-retention-days"] {
            let output = fixture.plan(&[flag, "0"]);
            assert_eq!(output.code, 64, "{flag}: {}", output.stderr_text());
        }
    }

    #[test]
    fn apply_deletes_only_idle_scratch_runs() {
        let fixture = Fixture::new();
        let old = fixture.make_run("out/scratch", Some("owner__repo"), OLD_RUN);
        age_tree(&old);
        let today = fixture.make_run("out/scratch", Some("owner__repo"), &today_run_id());

        let plan = fixture.plan(&[]).stdout_json();
        let applied = fixture.apply(&plan);
        assert_eq!(applied.code, 0, "stdout={}", applied.stdout_text());
        assert_eq!(applied.stdout_json()["result"]["summary"]["deleted"], 1);
        assert!(!old.exists());
        assert!(today.exists());
    }

    #[test]
    fn plan_and_apply_reclaim_idle_run_builds_without_an_out_root() {
        let fixture = Fixture::new();
        let old = fixture.make_run("build/run", None, OLD_RUN);
        fs::create_dir_all(old.join("debug")).unwrap();
        fs::write(old.join("debug/artifact"), "bytes").unwrap();
        age_tree(&old);
        let fresh = fixture.make_run("build/run", None, &today_run_id());

        let plan = fixture.plan(&[]).stdout_json();
        assert_eq!(plan["result"]["out_root_exists"], false);
        let old_row = row(&plan, OLD_RUN);
        assert_eq!(old_row["category"], "run-build");
        assert_eq!(old_row["action"], "delete");
        assert_eq!(row(&plan, &today_run_id())["action"], "preserve");

        let applied = fixture.apply(&plan);
        assert_eq!(applied.code, 0, "stdout={}", applied.stdout_text());
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn apply_refuses_forged_class_delete_shapes() {
        let fixture = Fixture::new();
        let old = fixture.make_run("out/scratch", Some("owner__repo"), OLD_RUN);
        let build = fixture.make_run("build/run", None, OLD_RUN);
        let stray = fixture.make_run("build", Some("other"), OLD_RUN);
        for dir in [&old, &build, &stray] {
            age_tree(dir);
        }
        let plan = fixture.plan(&[]).stdout_json();

        let forge = |category: &str, path: &Path| {
            let mut forged = plan.clone();
            let mut item = forged["result"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["category"] == category)
                .unwrap_or_else(|| panic!("no {category} row"))
                .clone();
            item["path"] = Value::String(path.to_string_lossy().to_string());
            forged["result"]["items"] = Value::Array(vec![item]);
            resign(&mut forged);
            fixture.apply(&forged)
        };

        // A scratch row pointing at the repo directory instead of a run.
        let shallow = forge("scratch-artifact", old.parent().unwrap());
        assert_eq!(shallow.code, 65, "stdout={}", shallow.stdout_text());
        // A run-build row outside build/run.
        let outside = forge("run-build", &stray);
        assert_eq!(outside.code, 65, "stdout={}", outside.stdout_text());
        assert!(old.exists() && build.exists() && stray.exists());
    }

    #[test]
    fn plan_flags_build_trees_inside_evidence_runs() {
        let fixture = Fixture::new();
        let built = fixture.make_run("out/projects", Some("owner__repo"), "20200101-000000-built");
        fs::create_dir_all(built.join("target/debug")).unwrap();
        fs::write(built.join("target/.rustc_info.json"), "{}").unwrap();
        let tagged = fixture.make_run(
            "out/projects",
            Some("owner__repo"),
            "20200101-000000-tagged",
        );
        fs::create_dir_all(tagged.join("cache")).unwrap();
        fs::write(
            tagged.join("cache/CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n# cache dir\n",
        )
        .unwrap();
        let plain = fixture.make_run("out/projects", Some("owner__repo"), "20200101-000000-plain");
        let lookalike =
            fixture.make_run("out/projects", Some("owner__repo"), "20200101-000000-look");
        fs::write(lookalike.join("logs/CACHEDIR.TAG"), "not a cache tag\n").unwrap();
        for dir in [&built, &tagged, &plain, &lookalike] {
            age_tree(dir);
        }

        let plan = fixture.plan(&["--include-projects"]).stdout_json();
        for name in ["20200101-000000-built", "20200101-000000-tagged"] {
            let flagged = row(&plan, name);
            assert_eq!(flagged["contains_build_tree"], true, "{name}");
            assert!(
                flagged["reason"].as_str().unwrap().contains("build tree"),
                "{name}"
            );
            assert_eq!(flagged["action"], "needs-policy", "{name}");
        }
        for name in ["20200101-000000-plain", "20200101-000000-look"] {
            assert!(
                row(&plan, name).get("contains_build_tree").is_none(),
                "{name}"
            );
        }
        assert_eq!(plan["result"]["summary"]["build_trees"], 2);

        let home = fixture.home_arg();
        let text = run(
            fixture.tmp.path(),
            &[
                "cleanup",
                "plan",
                "--agent-home",
                &home,
                "--include-projects",
            ],
        );
        assert!(
            text.stdout_text().contains("build trees in evidence: 2"),
            "{}",
            text.stdout_text()
        );
    }
}
