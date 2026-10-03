//! Fragment layout contracts exercised through the binary and real Git.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nils_test_support::{bin, tempdir::ScopedTempDir};
use pretty_assertions::assert_eq;

const INDEX: &str = "# Development log\n\nConventions.\n\n## Months\n\n";

struct Repo {
    _temp: ScopedTempDir,
    root: PathBuf,
}
impl Repo {
    fn new() -> Self {
        let temp = ScopedTempDir::with_prefix("devlog-fragments-");
        let root = temp.path().canonicalize().unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Test Maintainer"]);
        git(
            &root,
            &["config", "user.email", "maintainer@example.invalid"],
        );
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        std::fs::create_dir_all(root.join("docs/devlog")).unwrap();
        std::fs::write(root.join("docs/devlog/README.md"), INDEX).unwrap();
        commit(&root, "Initialize log");
        Self { _temp: temp, root }
    }
    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join("docs/devlog").join(path)).unwrap()
    }
}
fn git(root: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn commit(root: &Path, message: &str) {
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", message]);
}
fn run(root: &Path, fragments: bool, args: &[&str]) -> Output {
    let mut command = Command::new(bin::resolve("devlog"));
    command.current_dir(root).env_remove("DEVLOG_LAYOUT");
    if fragments {
        command.env("DEVLOG_LAYOUT", "fragments");
    }
    command.args(args).output().unwrap()
}
fn success(root: &Path, args: &[&str]) -> Output {
    let out = run(root, true, args);
    assert!(
        out.status.success(),
        "{args:?}: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
fn new(root: &Path, title: &str, date: &str, slug: &str) {
    success(
        root,
        &[
            "new",
            "--title",
            title,
            "--date",
            date,
            "--slug",
            slug,
            "--result",
            "Shipped.",
            "--why",
            "Needed.",
            "--evidence",
            "Verified.",
        ],
    );
}
#[test]
fn fragment_new_never_changes_shared_files_and_readers_find_it() {
    let repo = Repo::new();
    // No --slug: the opt-in writer must isolate even an ordinary invocation.
    success(
        &repo.root,
        &["new", "--title", "Isolated change", "--date", "2020-04-20"],
    );
    assert_eq!(repo.read("README.md"), INDEX);
    assert!(
        !repo.root.join("docs/devlog/2020-04.md").exists(),
        "fragment new must not create a month file"
    );
    let check = success(&repo.root, &["--format", "json", "check"]);
    let json: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(json["data"]["entry_count"], 1);
    let search = success(&repo.root, &["search", "Isolated", "--month", "2020-04"]);
    assert!(String::from_utf8_lossy(&search.stdout).contains("pending/"));
    success(&repo.root, &["index"]);
    assert!(repo.read("README.md").contains("pending/"));
    success(&repo.root, &["check"]);
}
#[test]
fn default_new_preserves_exact_output_and_file_bytes() {
    let repo = Repo::new();
    let out = run(
        &repo.root,
        false,
        &["new", "--title", "Default entry", "--date", "2020-04-20"],
    );
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "added 2020-04-20 to docs/devlog/2020-04.md\ncreated docs/devlog/2020-04.md\nupdated docs/devlog/README.md\n"
    );
    assert_eq!(
        repo.read("2020-04.md"),
        "# Development log - 2020-04\n\n## 2020-04-20 - Default entry\n\n### Result\n\n- TODO\n\n### Why / context\n\n- TODO\n\n### Evidence\n\n- TODO\n"
    );
    assert_eq!(
        repo.read("README.md"),
        "# Development log\n\nConventions.\n\n## Months\n\n- [2020-04](2020-04.md)\n"
    );
    let before = repo.read("2020-04.md");
    let out = run(
        &repo.root,
        false,
        &[
            "--format",
            "json",
            "new",
            "--title",
            "Second",
            "--date",
            "2020-04-21",
        ],
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"schema_version\":\"cli.devlog.new.v1\",\"ok\":true,\"data\":{\"month\":\"2020-04\",\"date\":\"2020-04-21\",\"path\":\"docs/devlog/2020-04.md\",\"created_month_file\":false,\"index_updated\":false}}\n"
    );
    assert!(
        repo.read("2020-04.md")
            .ends_with(before.split_once("\n\n").unwrap().1)
    );
    assert!(!repo.root.join("docs/devlog/pending").exists());
}
#[test]
fn parallel_fragment_branches_merge_cleanly_in_either_order() {
    for first in ["alpha", "beta"] {
        let repo = Repo::new();
        for branch in ["alpha", "beta"] {
            git(&repo.root, &["checkout", "-qb", branch, "main"]);
            new(&repo.root, "Same title", "2020-04-20", branch);
            commit(&repo.root, "Add fragment");
        }
        git(&repo.root, &["checkout", "-q", "main"]);
        git(&repo.root, &["merge", "--no-edit", first]);
        let second = if first == "alpha" { "beta" } else { "alpha" };
        git(&repo.root, &["merge", "--no-edit", second]);
        assert_eq!(repo.read("README.md"), INDEX);
        assert!(
            repo.root
                .join("docs/devlog/pending/2020-04-20-alpha.md")
                .is_file()
        );
        assert!(
            repo.root
                .join("docs/devlog/pending/2020-04-20-beta.md")
                .is_file()
        );
        success(&repo.root, &["check"]);
    }
}
#[test]
fn fold_is_identical_in_two_clones_idempotent_and_keeps_today() {
    let repo = Repo::new();
    new(&repo.root, "Beta", "2020-04-20", "beta");
    new(&repo.root, "Alpha", "2020-04-20", "alpha");
    new(&repo.root, "Older", "2020-03-01", "older");
    let today = nils_devlog::EntryDate::today().unwrap().to_string();
    new(&repo.root, "Today", &today, "today");
    commit(&repo.root, "Pending entries");
    let copies = ScopedTempDir::with_prefix("devlog-clones-");
    let a = copies.path().join("a");
    let b = copies.path().join("b");
    for clone in [&a, &b] {
        git(
            copies.path(),
            &[
                "clone",
                "-q",
                repo.root.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        success(clone, &["fold"]);
        success(clone, &["check"]);
    }
    for name in ["2020-04.md", "2020-03.md", "README.md"] {
        let path = Path::new("docs/devlog").join(name);
        assert_eq!(
            std::fs::read(a.join(&path)).unwrap(),
            std::fs::read(b.join(&path)).unwrap()
        );
    }
    let month = std::fs::read_to_string(a.join("docs/devlog/2020-04.md")).unwrap();
    assert!(month.find("Alpha").unwrap() < month.find("Beta").unwrap());
    assert!(
        a.join(format!("docs/devlog/pending/{today}-today.md"))
            .exists()
    );
    let before = git(&a, &["diff"]).stdout;
    let second = success(&a, &["--format", "json", "fold"]);
    let json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(json["data"]["folded"], 0);
    assert_eq!(git(&a, &["diff"]).stdout, before);
}
#[test]
fn check_rejects_baseline_fragment_edits_and_only_accepts_exact_fold_deletion() {
    let repo = Repo::new();
    new(&repo.root, "Original", "2020-04-20", "original");
    commit(&repo.root, "Immutable entry");
    git(&repo.root, &["checkout", "-qb", "change"]);
    let path = repo.root.join("docs/devlog/pending/2020-04-20-original.md");
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, original.replace("Shipped.", "Edited.")).unwrap();
    let out = run(&repo.root, true, &["check"]);
    assert_eq!(out.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&out.stdout).contains("fragment-modified"));
    std::fs::remove_file(&path).unwrap();
    let out = run(&repo.root, true, &["check"]);
    assert_eq!(out.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&out.stdout).contains("fragment-deleted"));
    std::fs::write(&path, &original).unwrap();
    success(&repo.root, &["fold"]);
    success(&repo.root, &["check"]);
    let month = repo.root.join("docs/devlog/2020-04.md");
    let folded = std::fs::read_to_string(&month).unwrap();
    std::fs::write(month, folded.replace("Shipped.", "Edited.")).unwrap();
    assert_eq!(run(&repo.root, true, &["check"]).status.code(), Some(65));
}
#[test]
fn git_merge_driver_unions_independently_folded_entries() {
    let repo = Repo::new();
    new(&repo.root, "Shared", "2020-04-01", "shared");
    success(&repo.root, &["fold"]);
    std::fs::write(
        repo.root.join(".gitattributes"),
        "docs/devlog/????-??.md merge=devlog\n",
    )
    .unwrap();
    let binary = bin::resolve("devlog").canonicalize().unwrap();
    git(
        &repo.root,
        &[
            "config",
            "merge.devlog.driver",
            &format!("'{}' merge %O %A %B", binary.display()),
        ],
    );
    commit(&repo.root, "Configure driver");
    for branch in ["alpha", "beta"] {
        git(&repo.root, &["checkout", "-qb", branch, "main"]);
        new(&repo.root, branch, "2020-04-20", branch);
        success(&repo.root, &["fold"]);
        commit(&repo.root, "Fold entry");
    }
    git(&repo.root, &["checkout", "-q", "alpha"]);
    git(&repo.root, &["merge", "--no-edit", "beta"]);
    let month = repo.read("2020-04.md");
    for title in ["Shared", "alpha", "beta"] {
        assert_eq!(month.matches(&format!(" - {title}\n")).count(), 1);
    }
    success(&repo.root, &["check"]);
}

#[test]
fn generated_names_isolate_same_title_and_explicit_slug_cannot_overwrite() {
    let repo = Repo::new();
    for _ in 0..2 {
        success(
            &repo.root,
            &["new", "--title", "Same title", "--date", "2020-04-20"],
        );
    }
    assert_eq!(
        std::fs::read_dir(repo.root.join("docs/devlog/pending"))
            .unwrap()
            .count(),
        2
    );
    new(&repo.root, "Explicit", "2020-04-20", "explicit");
    let before = repo.read("pending/2020-04-20-explicit.md");
    let out = run(
        &repo.root,
        true,
        &[
            "new",
            "--title",
            "Overwrite",
            "--date",
            "2020-04-20",
            "--slug",
            "explicit",
        ],
    );
    assert!(!out.status.success());
    assert_eq!(repo.read("pending/2020-04-20-explicit.md"), before);
}
#[test]
fn fold_validates_the_entire_set_before_writing_and_today_only_is_no_op() {
    let repo = Repo::new();
    let today = nils_devlog::EntryDate::today().unwrap().to_string();
    new(&repo.root, "Today", &today, "today");
    success(&repo.root, &["fold"]);
    assert_eq!(repo.read("README.md"), INDEX);
    new(&repo.root, "Valid", "2020-04-20", "valid");
    let invalid = repo.root.join("docs/devlog/pending/2020-05-01-invalid.md");
    std::fs::write(&invalid, "not an entry\n").unwrap();
    let out = run(&repo.root, true, &["check"]);
    assert_eq!(out.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&out.stdout).contains("invalid-fragment"));
    assert_eq!(run(&repo.root, true, &["fold"]).status.code(), Some(65));
    assert!(!repo.root.join("docs/devlog/2020-04.md").exists());
    assert!(
        repo.root
            .join("docs/devlog/pending/2020-04-20-valid.md")
            .exists()
    );
    assert_eq!(repo.read("README.md"), INDEX);
}
#[test]
fn merge_keeps_one_sided_corrections_and_refuses_incompatible_corrections() {
    let repo = Repo::new();
    new(&repo.root, "Original", "2020-04-20", "original");
    success(&repo.root, &["fold"]);
    let base_text = repo.read("2020-04.md");
    let base = repo.root.join("base");
    let ours = repo.root.join("ours");
    let theirs = repo.root.join("theirs");
    std::fs::write(&base, &base_text).unwrap();
    std::fs::write(&ours, base_text.replace("Shipped.", "Corrected.")).unwrap();
    std::fs::write(&theirs, &base_text).unwrap();
    success(
        &repo.root,
        &[
            "merge",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
        ],
    );
    let corrected = std::fs::read_to_string(&ours).unwrap();
    assert!(corrected.contains("Corrected."));
    std::fs::write(&theirs, base_text.replace("Shipped.", "Other correction.")).unwrap();
    let out = run(
        &repo.root,
        true,
        &[
            "merge",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(65));
    assert_eq!(std::fs::read_to_string(&ours).unwrap(), corrected);
}
#[test]
fn check_detects_deletion_of_the_entire_pending_directory_and_supports_explicit_base() {
    let repo = Repo::new();
    new(&repo.root, "Immutable", "2020-04-20", "immutable");
    commit(&repo.root, "Baseline fragment");
    git(&repo.root, &["checkout", "-qb", "change"]);
    std::fs::remove_dir_all(repo.root.join("docs/devlog/pending")).unwrap();
    let out = run(&repo.root, true, &["check", "--base", "main"]);
    assert_eq!(out.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&out.stdout).contains("fragment-deleted"));
}

#[test]
fn default_index_preserves_repository_pending_prose_byte_for_byte() {
    let repo = Repo::new();
    let out = run(
        &repo.root,
        false,
        &["new", "--title", "Month entry", "--date", "2020-04-20"],
    );
    assert!(out.status.success());
    let index = repo.read("README.md").trim_end().to_string()
        + "\n\n## Pending\n\n- Repository planning prose.\n\n## Notes\n\nKeep these notes.\n";
    std::fs::write(repo.root.join("docs/devlog/README.md"), &index).unwrap();
    let out = run(&repo.root, false, &["index"]);
    assert!(out.status.success());
    assert_eq!(repo.read("README.md"), index);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "index already current: 1 months\n"
    );
    let out = run(&repo.root, false, &["check"]);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "docs/devlog: 1 months, 1 entries\nok: no structural problems\n"
    );
    let out = run(&repo.root, false, &["search", "Month entry"]);
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "2020-04.md:3:## 2020-04-20 - Month entry\n"
    );
}
#[test]
fn relative_dir_check_enforces_the_default_branch_baseline() {
    let repo = Repo::new();
    new(&repo.root, "Immutable", "2020-04-20", "immutable");
    commit(&repo.root, "Baseline fragment");
    git(&repo.root, &["checkout", "-qb", "change"]);
    let path = repo
        .root
        .join("docs/devlog/pending/2020-04-20-immutable.md");
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, original.replace("Shipped.", "Edited.")).unwrap();
    let out = run(
        &repo.root,
        true,
        &["--dir", "docs/devlog", "check", "--base", "main"],
    );
    assert_eq!(out.status.code(), Some(65));
    assert!(String::from_utf8_lossy(&out.stdout).contains("fragment-modified"));
}
#[test]
fn git_driver_unions_two_branches_adding_the_first_month_file() {
    let repo = Repo::new();
    std::fs::write(
        repo.root.join(".gitattributes"),
        "docs/devlog/????-??.md merge=devlog\n",
    )
    .unwrap();
    let binary = bin::resolve("devlog").canonicalize().unwrap();
    git(
        &repo.root,
        &[
            "config",
            "merge.devlog.driver",
            &format!("'{}' merge %O %A %B", binary.display()),
        ],
    );
    commit(&repo.root, "Configure driver");
    for branch in ["alpha", "beta"] {
        git(&repo.root, &["checkout", "-qb", branch, "main"]);
        new(&repo.root, "Same title", "2020-04-20", branch);
        success(&repo.root, &["fold"]);
        commit(&repo.root, "First month entry");
    }
    git(&repo.root, &["checkout", "-q", "alpha"]);
    git(&repo.root, &["merge", "--no-edit", "beta"]);
    let month = repo.read("2020-04.md");
    for slug in ["alpha", "beta"] {
        assert_eq!(
            month
                .matches(&format!("<!-- devlog-id: 2020-04-20-{slug} -->"))
                .count(),
            1
        );
    }
    assert_eq!(month.matches("- Shipped.").count(), 2);
    assert!(month.find("-alpha -->").unwrap() < month.find("-beta -->").unwrap());
    success(&repo.root, &["check"]);
}

#[cfg(unix)]
#[test]
fn failed_fold_index_update_keeps_fragments_for_a_convergent_retry() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    new(&repo.root, "Recoverable", "2020-04-20", "recoverable");
    let index = repo.root.join("docs/devlog/README.md");
    std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o444)).unwrap();
    // A privileged runner can write a read-only file; this permission-based
    // failure scenario exists only when the filesystem enforces its mode.
    if std::fs::OpenOptions::new().write(true).open(&index).is_ok() {
        std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    let out = run(&repo.root, true, &["fold"]);
    std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!out.status.success());
    assert!(
        repo.root
            .join("docs/devlog/pending/2020-04-20-recoverable.md")
            .exists(),
        "failed index update must retain the source fragment"
    );
    success(&repo.root, &["fold"]);
    assert_eq!(
        repo.read("2020-04.md").matches("- Recoverable\n").count(),
        1
    );
    assert!(repo.read("README.md").contains("- [2020-04](2020-04.md)"));
    assert!(
        !repo
            .root
            .join("docs/devlog/pending/2020-04-20-recoverable.md")
            .exists()
    );
    success(&repo.root, &["check"]);
}

fn strict_check(root: &Path, dir: Option<&str>, base: &str) -> Output {
    let mut args = vec!["--format", "json"];
    if let Some(dir) = dir {
        args.extend(["--dir", dir]);
    }
    args.extend(["check", "--base", base, "--fragments-only"]);
    run(root, true, &args)
}

fn assert_problem(out: &Output, kind: &str, path: &str) {
    assert_eq!(
        out.status.code(),
        Some(65),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema_version"], "cli.devlog.check.v1");
    assert_eq!(json["ok"], false);
    assert!(
        json["error"]["details"]["problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["kind"] == kind && p["path"] == path),
        "{json}"
    );
}

#[test]
fn fragments_only_rejects_month_changes_in_all_git_states_and_log_locations() {
    for (dir, explicit) in [
        ("docs/devlog", false),
        ("docs/source/devlog", false),
        ("history/log", true),
    ] {
        for state in ["committed", "staged", "unstaged"] {
            for change in ["added", "modified", "renamed", "deleted"] {
                let repo = Repo::new();
                new(&repo.root, "Baseline", "2020-04-20", "baseline");
                success(&repo.root, &["fold"]);
                if dir != "docs/devlog" {
                    std::fs::create_dir_all(repo.root.join(dir).parent().unwrap()).unwrap();
                    std::fs::rename(repo.root.join("docs/devlog"), repo.root.join(dir)).unwrap();
                }
                commit(&repo.root, "Baseline month");
                git(&repo.root, &["checkout", "-qb", "feature"]);
                let month = repo.root.join(dir).join("2020-04.md");
                let original = std::fs::read_to_string(&month).unwrap();
                match change {
                    "added" => {
                        std::fs::write(
                            repo.root.join(dir).join("2020-03.md"),
                            "# Development log - 2020-03\n",
                        )
                        .unwrap();
                    }
                    "modified" => {
                        std::fs::write(&month, original.replace("Shipped.", "Edited.")).unwrap();
                    }
                    "renamed" => {
                        std::fs::rename(&month, repo.root.join(dir).join("archived.md")).unwrap();
                    }
                    "deleted" => {
                        std::fs::remove_file(&month).unwrap();
                    }
                    _ => unreachable!(),
                }
                match state {
                    "committed" => commit(&repo.root, "Change month"),
                    "staged" => {
                        git(&repo.root, &["add", "."]);
                    }
                    _ => {}
                }
                let expected = if change == "added" {
                    "2020-03.md"
                } else {
                    "2020-04.md"
                };
                assert_problem(
                    &strict_check(&repo.root, explicit.then_some(dir), "main"),
                    "month-file-changed",
                    &format!("{dir}/{expected}"),
                );
            }
        }
    }
}

#[test]
fn fragments_only_rejects_local_fold_but_trusted_fold_commit_passes_ordinary_check() {
    let repo = Repo::new();
    new(&repo.root, "Merged", "2020-04-20", "merged");
    commit(&repo.root, "Merged fragment");
    let baseline = String::from_utf8(git(&repo.root, &["rev-parse", "HEAD"]).stdout).unwrap();
    git(&repo.root, &["checkout", "-qb", "feature"]);
    success(&repo.root, &["fold"]);
    for committed in [false, true] {
        if committed {
            commit(&repo.root, "Local fold");
        }
        assert_problem(
            &strict_check(&repo.root, None, "main"),
            "fragment-deleted",
            "docs/devlog/pending/2020-04-20-merged.md",
        );
        assert_problem(
            &strict_check(&repo.root, None, "main"),
            "month-file-changed",
            "docs/devlog/2020-04.md",
        );
        success(&repo.root, &["check", "--base", "main"]);
    }
    git(&repo.root, &["checkout", "-q", "main"]);
    success(&repo.root, &["fold"]);
    success(&repo.root, &["check", "--base", baseline.trim()]);
    commit(&repo.root, "Scheduled fold");
    success(&repo.root, &["check", "--base", baseline.trim()]);
}

#[test]
fn fragments_only_uses_merge_base_when_main_advances_and_accepts_new_fragments() {
    let repo = Repo::new();
    new(&repo.root, "Merged", "2020-04-20", "merged");
    commit(&repo.root, "Merged fragment");
    git(&repo.root, &["checkout", "-qb", "feature"]);
    new(&repo.root, "New", "2020-04-21", "new");
    commit(&repo.root, "New fragment");
    git(&repo.root, &["checkout", "-q", "main"]);
    success(&repo.root, &["fold"]);
    new(&repo.root, "Main only", "2020-04-22", "main-only");
    commit(&repo.root, "Main advanced");
    git(&repo.root, &["checkout", "-q", "feature"]);
    assert!(strict_check(&repo.root, None, "main").status.success());
    let path = repo.root.join("docs/devlog/pending/2020-04-20-merged.md");
    let old = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, old.replace("Shipped.", "Edited.")).unwrap();
    assert_problem(
        &strict_check(&repo.root, None, "main"),
        "fragment-modified",
        "docs/devlog/pending/2020-04-20-merged.md",
    );
}

#[test]
fn fragments_only_fails_closed_without_a_resolvable_merge_base() {
    let repo = Repo::new();
    for base in ["missing-ref", "main:docs/devlog"] {
        let out = strict_check(&repo.root, None, base);
        assert_eq!(out.status.code(), Some(69));
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["error"]["code"], "baseline-unavailable");
    }
    git(&repo.root, &["checkout", "--orphan", "unrelated"]);
    commit(&repo.root, "Unrelated history");
    assert_eq!(
        strict_check(&repo.root, None, "main").status.code(),
        Some(69)
    );
    assert_eq!(
        run(&repo.root, true, &["check", "--fragments-only"])
            .status
            .code(),
        Some(64)
    );
}

#[test]
fn fragments_only_checks_index_and_head_even_when_worktree_restores_baseline() {
    for committed in [false, true] {
        let repo = Repo::new();
        new(&repo.root, "Baseline", "2020-04-20", "baseline");
        success(&repo.root, &["fold"]);
        commit(&repo.root, "Baseline month");
        git(&repo.root, &["checkout", "-qb", "feature"]);
        let month = repo.root.join("docs/devlog/2020-04.md");
        let original = std::fs::read_to_string(&month).unwrap();
        std::fs::write(&month, original.replace("Shipped.", "Edited.")).unwrap();
        git(&repo.root, &["add", "."]);
        if committed {
            commit(&repo.root, "Change month");
        }
        std::fs::write(&month, &original).unwrap();
        assert_problem(
            &strict_check(&repo.root, None, "main"),
            "month-file-changed",
            "docs/devlog/2020-04.md",
        );
        assert!(
            run(&repo.root, true, &["check", "--base", "main"])
                .status
                .success()
        );
    }
}

#[test]
fn fragments_only_checks_merged_fragment_changes_hidden_by_a_later_git_layer() {
    for committed in [false, true] {
        let repo = Repo::new();
        new(&repo.root, "Merged", "2020-04-20", "merged");
        commit(&repo.root, "Merged fragment");
        git(&repo.root, &["checkout", "-qb", "feature"]);
        let path = repo.root.join("docs/devlog/pending/2020-04-20-merged.md");
        let original = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, original.replace("Shipped.", "Edited.")).unwrap();
        git(&repo.root, &["add", "."]);
        if committed {
            commit(&repo.root, "Change fragment");
        }
        std::fs::write(&path, original).unwrap();
        assert_problem(
            &strict_check(&repo.root, None, "main"),
            "fragment-modified",
            "docs/devlog/pending/2020-04-20-merged.md",
        );
    }
}

#[test]
fn fragments_only_passes_new_fragments_without_requiring_the_writer_environment() {
    for state in ["unstaged", "staged", "committed"] {
        let repo = Repo::new();
        git(&repo.root, &["checkout", "-qb", "feature"]);
        new(&repo.root, "New", "2020-04-20", "new");
        if state != "unstaged" {
            git(&repo.root, &["add", "."]);
        }
        if state == "committed" {
            commit(&repo.root, "New fragment");
        }
        let out = run(
            &repo.root,
            false,
            &["check", "--base", "main", "--fragments-only"],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let ordinary = run(&repo.root, false, &["check"]);
        assert_eq!(out.stdout, ordinary.stdout);
        assert_eq!(out.stderr, ordinary.stderr);
    }
}

#[test]
fn fragments_only_cannot_hide_baseline_deletions_by_switching_log_conventions() {
    for (old_dir, new_dir) in [
        ("docs/devlog", "docs/source/devlog"),
        ("docs/source/devlog", "docs/devlog"),
    ] {
        let repo = Repo::new();
        new(&repo.root, "Folded", "2020-04-20", "folded");
        success(&repo.root, &["fold"]);
        new(&repo.root, "Merged", "2020-04-21", "merged");
        if old_dir != "docs/devlog" {
            std::fs::create_dir_all(repo.root.join(old_dir).parent().unwrap()).unwrap();
            std::fs::rename(repo.root.join("docs/devlog"), repo.root.join(old_dir)).unwrap();
        }
        commit(&repo.root, "Baseline log");
        git(&repo.root, &["checkout", "-qb", "feature"]);
        std::fs::remove_dir_all(repo.root.join(old_dir)).unwrap();
        std::fs::create_dir_all(repo.root.join(new_dir)).unwrap();
        std::fs::write(repo.root.join(new_dir).join("README.md"), INDEX).unwrap();
        for committed in [false, true] {
            if committed {
                commit(&repo.root, "Switch log directory");
            }
            for dir in [None, Some(new_dir)] {
                let out = strict_check(&repo.root, dir, "main");
                assert_problem(&out, "month-file-changed", &format!("{old_dir}/2020-04.md"));
                assert_problem(
                    &out,
                    "fragment-deleted",
                    &format!("{old_dir}/pending/2020-04-21-merged.md"),
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn fragments_only_cannot_hide_custom_log_deletions_by_retargeting_a_symlink() {
    let repo = Repo::new();
    new(&repo.root, "Folded", "2020-04-20", "folded");
    success(&repo.root, &["fold"]);
    new(&repo.root, "Merged", "2020-04-21", "merged");
    std::fs::create_dir_all(repo.root.join("history")).unwrap();
    std::fs::rename(repo.root.join("docs/devlog"), repo.root.join("history/log")).unwrap();
    commit(&repo.root, "Baseline custom log");
    git(&repo.root, &["checkout", "-qb", "feature"]);
    std::fs::remove_dir_all(repo.root.join("history/log")).unwrap();
    std::fs::create_dir_all(repo.root.join("history/other")).unwrap();
    std::fs::write(repo.root.join("history/other/README.md"), INDEX).unwrap();
    std::os::unix::fs::symlink("other", repo.root.join("history/log")).unwrap();
    for committed in [false, true] {
        if committed {
            commit(&repo.root, "Retarget custom log");
        }
        let out = strict_check(&repo.root, Some("history/log"), "main");
        assert_problem(&out, "month-file-changed", "history/log/2020-04.md");
        assert_problem(
            &out,
            "fragment-deleted",
            "history/log/pending/2020-04-21-merged.md",
        );
    }
}

#[cfg(unix)]
fn baseline_symlink_retarget(committed: bool, ancestor: bool) {
    let repo = Repo::new();
    new(&repo.root, "Folded", "2020-04-20", "folded");
    success(&repo.root, &["fold"]);
    new(&repo.root, "Merged", "2020-04-21", "merged");
    let (old_dir, new_dir, link, selected) = if ancestor {
        (
            "history/old/log",
            "history/new/log",
            "history/link",
            "history/link/log",
        )
    } else {
        ("history/old", "history/new", "history/log", "history/log")
    };
    std::fs::create_dir_all(repo.root.join(old_dir).parent().unwrap()).unwrap();
    std::fs::rename(repo.root.join("docs/devlog"), repo.root.join(old_dir)).unwrap();
    std::os::unix::fs::symlink("old", repo.root.join(link)).unwrap();
    commit(&repo.root, "Baseline symlink log");
    git(&repo.root, &["checkout", "-qb", "feature"]);
    // The original symlink and its target are supported while unchanged.
    assert!(
        strict_check(&repo.root, Some(selected), "main")
            .status
            .success()
    );
    std::fs::remove_dir_all(repo.root.join("history/old")).unwrap();
    std::fs::create_dir_all(repo.root.join(new_dir)).unwrap();
    std::fs::write(repo.root.join(new_dir).join("README.md"), INDEX).unwrap();
    std::fs::remove_file(repo.root.join(link)).unwrap();
    std::os::unix::fs::symlink("new", repo.root.join(link)).unwrap();
    if committed {
        commit(&repo.root, "Retarget baseline log symlink");
    }
    let out = strict_check(&repo.root, Some(selected), "main");
    assert_problem(&out, "log-path-changed", link);
    let ordinary = run(&repo.root, false, &["--dir", selected, "check"]);
    assert!(ordinary.status.success());
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_unstaged_baseline_symlink_retarget() {
    baseline_symlink_retarget(false, false);
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_committed_baseline_symlink_retarget() {
    baseline_symlink_retarget(true, false);
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_unstaged_baseline_symlink_ancestor_retarget() {
    baseline_symlink_retarget(false, true);
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_committed_baseline_symlink_ancestor_retarget() {
    baseline_symlink_retarget(true, true);
}

#[cfg(unix)]
fn baseline_symlink_intermediate_retarget(committed: bool, selected: &str) {
    let repo = Repo::new();
    new(&repo.root, "Folded", "2020-04-20", "folded");
    success(&repo.root, &["fold"]);
    new(&repo.root, "Merged", "2020-04-21", "merged");
    std::fs::create_dir_all(repo.root.join("history/nested/deeper")).unwrap();
    std::fs::write(repo.root.join("history/nested/deeper/.keep"), "").unwrap();
    std::fs::rename(
        repo.root.join("docs/devlog"),
        repo.root.join("history/nested/old"),
    )
    .unwrap();
    std::os::unix::fs::symlink("nested/deeper", repo.root.join("history/pivot")).unwrap();
    std::os::unix::fs::symlink("pivot/../old", repo.root.join("history/log")).unwrap();
    commit(&repo.root, "Baseline intermediate symlink log");
    git(&repo.root, &["checkout", "-qb", "feature"]);
    assert!(
        strict_check(&repo.root, Some(selected), "main")
            .status
            .success()
    );
    std::fs::remove_dir_all(repo.root.join("history/nested/old")).unwrap();
    std::fs::create_dir_all(repo.root.join("history/new/deeper")).unwrap();
    std::fs::write(repo.root.join("history/new/deeper/.keep"), "").unwrap();
    std::fs::create_dir_all(repo.root.join("history/new/old")).unwrap();
    std::fs::write(repo.root.join("history/new/old/README.md"), INDEX).unwrap();
    std::fs::remove_file(repo.root.join("history/pivot")).unwrap();
    std::os::unix::fs::symlink("new/deeper", repo.root.join("history/pivot")).unwrap();
    if committed {
        commit(&repo.root, "Retarget intermediate log symlink");
    }
    let out = strict_check(&repo.root, Some(selected), "main");
    assert_problem(&out, "log-path-changed", "history/pivot");
    assert_problem(&out, "month-file-changed", "history/nested/old/2020-04.md");
    assert_problem(
        &out,
        "fragment-deleted",
        "history/nested/old/pending/2020-04-21-merged.md",
    );
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_unstaged_baseline_symlink_intermediate_retarget() {
    baseline_symlink_intermediate_retarget(false, "history/log");
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_committed_baseline_symlink_intermediate_retarget() {
    baseline_symlink_intermediate_retarget(true, "history/log");
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_unstaged_baseline_symlink_in_parent_traversal_arg() {
    baseline_symlink_intermediate_retarget(false, "history/pivot/../old");
}

#[cfg(unix)]
#[test]
fn fragments_only_rejects_committed_baseline_symlink_in_parent_traversal_arg() {
    baseline_symlink_intermediate_retarget(true, "history/pivot/../old");
}
