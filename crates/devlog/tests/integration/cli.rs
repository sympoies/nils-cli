//! End-to-end coverage of the binary contract: exit codes, text output, and
//! the JSON envelope, against a real fixture repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use nils_test_support::bin;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_with};
use nils_test_support::tempdir::ScopedTempDir;
use pretty_assertions::assert_eq;

const INDEX: &str =
    "# Development log\n\nConventions live here.\n\n## Months\n\n- [2026-04](2026-04.md)\n";

struct Fixture {
    _dir: ScopedTempDir,
    root: PathBuf,
}

impl Fixture {
    /// A git repository containing a devlog at `dir` with one month file.
    fn new(devlog_dir: &str) -> Self {
        let dir = ScopedTempDir::with_prefix("devlog-cli-");
        // The process temp root can itself be a symlink; canonicalize so the
        // path the binary reports back matches what the test compares.
        let root = dir
            .path()
            .canonicalize()
            .expect("canonicalize fixture root");

        git(&root, &["init", "--quiet", "."]);

        let log = root.join(devlog_dir);
        std::fs::create_dir_all(&log).expect("create devlog dir");
        std::fs::write(log.join("README.md"), INDEX).expect("write index");
        std::fs::write(
            log.join("2026-04.md"),
            "# Development log - 2026-04\n\n## 2026-04-17 - Existing entry\n\n### Result\n\n- Did a thing.\n\n### Why / context\n\n- Because.\n\n### Evidence\n\n- Ran it.\n\n### Links\n\n- `abc12345`\n",
        )
        .expect("write month file");

        Self { _dir: dir, root }
    }

    fn devlog_path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.devlog_path(relative)).expect("read devlog file")
    }
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn run_in(root: &Path, args: &[&str]) -> CmdOutput {
    let binary = bin::resolve("devlog");
    let options = CmdOptions::new().with_cwd(root);
    run_with(&binary, args, &options)
}

#[test]
fn check_passes_on_a_well_formed_log() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert!(output.stdout_text().contains("no structural problems"));
}

#[test]
fn check_reports_data_exit_for_a_mis_named_month_file() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(fixture.devlog_path("docs/devlog/april.md"), "# nope\n")
        .expect("write mis-named file");

    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 65, "stderr={}", output.stderr_text());
    assert!(output.stdout_text().contains("unexpected-file"));
}

#[test]
fn check_reports_index_drift_in_both_directions() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-05.md"),
        "# Development log - 2026-05\n",
    )
    .expect("write unindexed month");

    let output = run_in(&fixture.root, &["--format", "json", "check"]);
    assert_eq!(output.code, 65);
    let json = output.stdout_json();
    let kinds: Vec<&str> = json["data"]["problems"]
        .as_array()
        .expect("problems array")
        .iter()
        .map(|problem| problem["kind"].as_str().expect("kind"))
        .collect();
    assert!(
        kinds.contains(&"index-missing-month"),
        "expected index drift to be reported, got {kinds:?}"
    );
}

#[test]
fn check_reports_entries_that_are_not_newest_first() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-01 - Older\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n\n## 2026-04-20 - Newer\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n",
    )
    .expect("write out-of-order month");

    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 65);
    assert!(output.stdout_text().contains("not-newest-first"));
}

#[test]
fn check_reports_a_missing_required_section() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - Thin entry\n\n### Result\n\n- only this\n",
    )
    .expect("write thin entry");

    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 65);
    let stdout = output.stdout_text();
    assert!(stdout.contains("missing-section"), "stdout={stdout}");
}

#[test]
fn new_inserts_newest_first_above_the_existing_entry() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "Later entry",
            "--date",
            "2026-04-20",
            "--result",
            "Shipped it.",
            "--why",
            "It was needed.",
            "--evidence",
            "Ran the gate.",
            "--link",
            "`abc12345`",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let contents = fixture.read("docs/devlog/2026-04.md");
    let later = contents
        .find("## 2026-04-20 - Later entry")
        .expect("new entry");
    let existing = contents
        .find("## 2026-04-17 - Existing entry")
        .expect("existing entry");
    assert!(
        later < existing,
        "new entry must be inserted above the older one"
    );
    assert!(contents.starts_with("# Development log - 2026-04\n"));

    // check must accept what new produced; a writer that emits a shape its own
    // checker rejects is the defect this pairing exists to catch.
    let verify = run_in(&fixture.root, &["check"]);
    assert_eq!(verify.code, 0, "stdout={}", verify.stdout_text());
}

#[test]
fn new_creates_a_missing_month_file_and_links_it_in_the_index() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &[
            "--format",
            "json",
            "new",
            "--title",
            "First of the month",
            "--date",
            "2026-05-02",
            "--result",
            "Shipped it.",
            "--why",
            "Needed.",
            "--evidence",
            "Gate passed.",
            "--link",
            "`abc12345`",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let json = output.stdout_json();
    assert_eq!(json["ok"], true);
    assert_eq!(json["schema_version"], "cli.devlog.new.v1");
    assert_eq!(json["data"]["created_month_file"], true);
    assert_eq!(json["data"]["index_updated"], true);

    let month = fixture.read("docs/devlog/2026-05.md");
    assert!(month.starts_with("# Development log - 2026-05\n"));

    let index = fixture.read("docs/devlog/README.md");
    assert!(index.contains("- [2026-05](2026-05.md)"));
    assert!(index.contains("- [2026-04](2026-04.md)"));
    // Prose around the index must survive a rewrite.
    assert!(index.contains("Conventions live here."));

    let verify = run_in(&fixture.root, &["check"]);
    assert_eq!(verify.code, 0, "stdout={}", verify.stdout_text());
}

#[test]
fn new_rejects_an_impossible_date_with_a_usage_exit() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "t",
            "--date",
            "2026-02-30",
            "--result",
            "r",
        ],
    );
    assert_eq!(output.code, 64, "stderr={}", output.stderr_text());
    assert!(output.stderr_text().contains("2026-02-30"));
}

#[test]
fn search_matches_case_insensitively_and_reports_line_numbers() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["search", "EXISTING"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let stdout = output.stdout_text();
    assert!(stdout.contains("2026-04.md:3:"), "stdout={stdout}");
}

#[test]
fn search_without_matches_exits_one_and_says_so_on_stderr() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["search", "nothing-matches-this"]);
    assert_eq!(output.code, 1);
    assert!(output.stderr_text().contains("no matches"));
    assert_eq!(output.stdout_text(), "");
}

#[test]
fn search_reports_an_absent_requested_month_distinctly() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["search", "anything", "--month", "2025-01"]);
    assert_eq!(output.code, 1);
    let stderr = output.stderr_text();
    assert!(
        stderr.contains("no devlog file for month") && stderr.contains("2025-01"),
        "an absent month must not read as an empty log: {stderr}"
    );
}

#[test]
fn search_rejects_a_malformed_month_with_a_usage_exit() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["search", "anything", "--month", "2026-13"]);
    assert_eq!(output.code, 64, "stderr={}", output.stderr_text());
}

#[test]
fn search_emits_a_json_envelope() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["--format", "json", "search", "existing"]);
    assert_eq!(output.code, 0);
    let json = output.stdout_json();
    assert_eq!(json["schema_version"], "cli.devlog.search.v1");
    assert_eq!(json["data"]["matches"][0]["month"], "2026-04");
    assert_eq!(json["data"]["matches"][0]["line_number"], 3);
}

#[test]
fn commands_run_from_a_subdirectory() {
    let fixture = Fixture::new("docs/devlog");
    let nested = fixture.root.join("docs");
    let output = run_in(&nested, &["check"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
}

#[test]
fn the_source_render_split_devlog_path_is_detected() {
    let fixture = Fixture::new("docs/source/devlog");
    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert!(output.stdout_text().contains("docs/source/devlog"));
}

#[test]
fn a_repository_without_a_devlog_reports_unavailable() {
    let dir = ScopedTempDir::with_prefix("devlog-cli-empty-");
    let root = dir.path().canonicalize().expect("canonicalize");
    git(&root, &["init", "--quiet", "."]);

    let output = run_in(&root, &["check"]);
    assert_eq!(output.code, 69, "stderr={}", output.stderr_text());
    assert!(output.stderr_text().contains("no development log"));
}

#[test]
fn index_is_idempotent() {
    let fixture = Fixture::new("docs/devlog");
    let first = run_in(&fixture.root, &["--format", "json", "index"]);
    assert_eq!(first.code, 0);

    let second = run_in(&fixture.root, &["--format", "json", "index"]);
    assert_eq!(second.code, 0);
    assert_eq!(second.stdout_json()["data"]["changed"], false);
}

#[test]
fn root_version_flag_is_available() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["-V"]);
    assert_eq!(output.code, 0);
    assert!(output.stdout_text().contains("devlog"));
}

#[test]
fn completion_exports_for_bash_and_zsh() {
    let fixture = Fixture::new("docs/devlog");
    for shell in ["bash", "zsh"] {
        let output = run_in(&fixture.root, &["completion", shell]);
        assert_eq!(output.code, 0, "shell={shell}");
        assert!(!output.stdout_text().is_empty(), "shell={shell}");
    }
}

#[test]
fn an_unknown_subcommand_emits_a_json_error_envelope_when_asked() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(&fixture.root, &["--format", "json", "nope"]);
    assert_eq!(output.code, 64);
    let json = output.stdout_json();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "unknown-subcommand");
}
