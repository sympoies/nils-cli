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
