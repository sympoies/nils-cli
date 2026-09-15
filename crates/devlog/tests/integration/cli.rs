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

/// Every problem kind the crate README documents, pinned against a fixture
/// that produces it. The classification is the product: `check` exists so these
/// are reportable, so an unreported or reclassified kind is the regression that
/// matters most here.
#[test]
fn every_documented_check_problem_kind_is_reported() {
    struct Case {
        kind: &'static str,
        month: &'static str,
        index: Option<&'static str>,
    }

    const GOOD_ENTRY: &str = "## 2026-04-17 - Title\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n";

    let cases = [
        Case {
            kind: "missing-heading",
            month: "# Wrong heading\n",
            index: None,
        },
        Case {
            kind: "malformed-entry-heading",
            month: "# Development log - 2026-04\n\n## no date here\n\n### Result\n\n- a\n",
            index: None,
        },
        Case {
            kind: "unknown-section",
            month: "# Development log - 2026-04\n\n## 2026-04-17 - Title\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n\n### Speculation\n\n- e\n",
            index: None,
        },
        Case {
            kind: "date-month-mismatch",
            month: "# Development log - 2026-04\n\n## 2026-05-02 - Wrong month\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n",
            index: None,
        },
        Case {
            kind: "conflict-markers",
            month: "# Development log - 2026-04\n\n<<<<<<< HEAD\n## 2026-04-17 - Title\n\n### Result\n\n- a\n>>>>>>> feature\n",
            index: None,
        },
        Case {
            kind: "index-stale-month",
            month: "",
            index: Some(
                "# Development log\n\n## Months\n\n- [2026-04](2026-04.md)\n- [2026-09](2026-09.md)\n",
            ),
        },
    ];

    for case in cases {
        let fixture = Fixture::new("docs/devlog");
        if !case.month.is_empty() {
            std::fs::write(fixture.devlog_path("docs/devlog/2026-04.md"), case.month)
                .expect("write month fixture");
        }
        if let Some(index) = case.index {
            std::fs::write(fixture.devlog_path("docs/devlog/README.md"), index)
                .expect("write index fixture");
            std::fs::write(
                fixture.devlog_path("docs/devlog/2026-04.md"),
                format!("# Development log - 2026-04\n\n{GOOD_ENTRY}"),
            )
            .expect("restore month fixture");
        }

        let output = run_in(&fixture.root, &["--format", "json", "check"]);
        assert_eq!(
            output.code,
            65,
            "kind={} stderr={}",
            case.kind,
            output.stderr_text()
        );
        let json = output.stdout_json();
        let kinds: Vec<String> = json["error"]["details"]["problems"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|problem| problem["kind"].as_str().map(str::to_string))
            .collect();
        assert!(
            kinds.iter().any(|reported| reported == case.kind),
            "expected {} to be reported, got {kinds:?}",
            case.kind
        );
    }
}

#[test]
fn a_log_without_an_index_reports_missing_index() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::remove_file(fixture.devlog_path("docs/devlog/README.md")).expect("remove index");

    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 65);
    assert!(output.stdout_text().contains("missing-index"));
}

#[test]
fn check_and_index_failure_envelopes_pin_their_schema_and_ok() {
    // The output contract requires one JSON snapshot per JSON-emitting
    // subcommand pinning the literal schema_version, and requires `ok` to
    // mirror the outcome rather than execution.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/nope.md"),
        "# not a month\n",
    )
    .expect("write mis-named file");

    let check = run_in(&fixture.root, &["--format", "json", "check"]);
    assert_eq!(check.code, 65);
    let json = check.stdout_json();
    assert_eq!(json["schema_version"], "cli.devlog.check.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "structural-problems");

    let index = run_in(&fixture.root, &["--format", "json", "index"]);
    assert_eq!(index.code, 0);
    let json = index.stdout_json();
    assert_eq!(json["schema_version"], "cli.devlog.index.v1");
    assert_eq!(json["ok"], true);
}

#[test]
fn a_search_without_matches_emits_a_failure_envelope() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &["--format", "json", "search", "absent-term"],
    );
    assert_eq!(output.code, 1);
    let json = output.stdout_json();
    assert_eq!(json["schema_version"], "cli.devlog.search.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "no-matches");
}

#[test]
fn index_drift_is_reported_in_both_directions() {
    let fixture = Fixture::new("docs/devlog");
    // A month file the index does not list, and an index entry with no file.
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-05.md"),
        "# Development log - 2026-05\n",
    )
    .expect("write unindexed month");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        "# Development log\n\n## Months\n\n- [2026-04](2026-04.md)\n- [2026-08](2026-08.md)\n",
    )
    .expect("write index listing a missing month");

    let output = run_in(&fixture.root, &["--format", "json", "check"]);
    assert_eq!(output.code, 65);
    let json = output.stdout_json();
    let kinds: Vec<String> = json["error"]["details"]["problems"]
        .as_array()
        .expect("problems array")
        .iter()
        .filter_map(|problem| problem["kind"].as_str().map(str::to_string))
        .collect();
    assert!(
        kinds.iter().any(|k| k == "index-missing-month"),
        "{kinds:?}"
    );
    assert!(kinds.iter().any(|k| k == "index-stale-month"), "{kinds:?}");
}

#[test]
fn index_sync_repairs_drift_and_preserves_surrounding_content() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-05.md"),
        "# Development log - 2026-05\n",
    )
    .expect("write unindexed month");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        "# Development log\n\nConventions live here.\n\n## Months\n\n- [2026-04](2026-04.md)\n\n## Afterword\n\nKeep me.\n",
    )
    .expect("write stale index");

    let first = run_in(&fixture.root, &["--format", "json", "index"]);
    assert_eq!(first.code, 0);
    assert_eq!(first.stdout_json()["data"]["changed"], true);

    let index = fixture.read("docs/devlog/README.md");
    assert!(index.contains("- [2026-05](2026-05.md)"));
    assert!(index.contains("Conventions live here."));
    assert!(
        index.contains("## Afterword") && index.contains("Keep me."),
        "content after the Months section must survive: {index}"
    );

    let second = run_in(&fixture.root, &["--format", "json", "index"]);
    assert_eq!(second.stdout_json()["data"]["changed"], false);
}

#[test]
fn a_months_heading_without_a_trailing_newline_is_not_duplicated() {
    // Regression: locating the section by substring missed a heading that ends
    // the file, so sync appended a second `## Months` and a later sync left
    // both in place permanently.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        "# Development log\n\n## Months",
    )
    .expect("write index with no trailing newline");

    let first = run_in(&fixture.root, &["index"]);
    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());

    let index = fixture.read("docs/devlog/README.md");
    assert_eq!(
        index.matches("## Months").count(),
        1,
        "exactly one Months heading must remain: {index}"
    );
    assert!(index.contains("- [2026-04](2026-04.md)"));

    let second = run_in(&fixture.root, &["--format", "json", "index"]);
    assert_eq!(second.stdout_json()["data"]["changed"], false);
}

#[test]
fn a_nested_months_subheading_is_not_mistaken_for_the_section() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        "# Development log\n\n### Months\n\nNot the section.\n\n## Months\n\n- [2026-04](2026-04.md)\n",
    )
    .expect("write index with a nested heading");

    let output = run_in(&fixture.root, &["index"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let index = fixture.read("docs/devlog/README.md");
    assert!(
        index.contains("### Months") && index.contains("Not the section."),
        "the nested subheading must be left alone: {index}"
    );
}

#[test]
fn new_refuses_a_month_file_whose_heading_is_wrong_and_leaves_it_untouched() {
    let fixture = Fixture::new("docs/devlog");
    let original = "# Development log - 2026-99\n\nsomething else\n";
    std::fs::write(fixture.devlog_path("docs/devlog/2026-04.md"), original)
        .expect("write bad-heading month");

    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "t",
            "--date",
            "2026-04-20",
            "--result",
            "r",
        ],
    );
    assert_eq!(output.code, 1, "stderr={}", output.stderr_text());
    assert!(output.stderr_text().contains("Development log - 2026-04"));
    assert_eq!(
        fixture.read("docs/devlog/2026-04.md"),
        original,
        "a refused insert must not modify the file"
    );
}

#[test]
fn new_without_an_explicit_date_uses_today() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &[
            "--format",
            "json",
            "new",
            "--title",
            "Dated today",
            "--result",
            "r",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    // Clock-tolerant: assert the reported month owns the reported date rather
    // than pinning a literal, so the test does not fail across a midnight run.
    let json = output.stdout_json();
    let month = json["data"]["month"].as_str().expect("month").to_string();
    let date = json["data"]["date"].as_str().expect("date").to_string();
    assert!(
        date.starts_with(&month),
        "date {date} must belong to month {month}"
    );
    assert!(
        fixture
            .devlog_path(&format!("docs/devlog/{month}.md"))
            .is_file()
    );
}

#[test]
fn an_explicit_dir_overrides_detection_and_a_non_directory_is_refused() {
    let fixture = Fixture::new("docs/source/devlog");
    let ok = run_in(&fixture.root, &["--dir", "docs/source/devlog", "check"]);
    assert_eq!(ok.code, 0, "stderr={}", ok.stderr_text());

    let bad = run_in(
        &fixture.root,
        &["--dir", "docs/source/devlog/README.md", "check"],
    );
    assert_eq!(bad.code, 69, "stderr={}", bad.stderr_text());
}

#[test]
fn a_missing_devlog_emits_a_json_error_envelope() {
    let dir = ScopedTempDir::with_prefix("devlog-cli-nolog-");
    let root = dir.path().canonicalize().expect("canonicalize");
    git(&root, &["init", "--quiet", "."]);

    let output = run_in(&root, &["--format", "json", "check"]);
    assert_eq!(output.code, 69);
    let json = output.stdout_json();
    assert_eq!(json["ok"], false);
    assert_eq!(json["schema_version"], "cli.devlog.error.v1");
    assert_eq!(json["error"]["code"], "devlog-not-found");
}

/// A month file left exactly as git writes it when two branches each added an
/// entry for the same month. Both sides are well-formed entries, which is why
/// the parser used to accept the file.
const CONFLICTED_MONTH: &str = "# Development log - 2026-04\n\n<<<<<<< HEAD\n## 2026-04-20 - Ours\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n=======\n## 2026-04-20 - Theirs\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n>>>>>>> feature\n\n## 2026-04-17 - Existing entry\n\n### Result\n\n- Did a thing.\n\n### Why / context\n\n- Because.\n\n### Evidence\n\n- Ran it.\n\n### Links\n\n- `abc12345`\n";

#[test]
fn check_reports_conflict_markers_in_a_month_file() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        CONFLICTED_MONTH,
    )
    .expect("write conflicted month");

    let output = run_in(&fixture.root, &["check"]);
    let stdout = output.stdout_text();
    assert_eq!(output.code, 65, "stdout={stdout}");
    assert!(stdout.contains("conflict-markers"), "stdout={stdout}");
}

#[test]
fn check_does_not_count_both_sides_of_a_conflict_as_entries() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        CONFLICTED_MONTH,
    )
    .expect("write conflicted month");

    let output = run_in(&fixture.root, &["--format", "json", "check"]);
    let envelope: serde_json::Value =
        serde_json::from_str(&output.stdout_text()).expect("parse envelope");
    assert_eq!(envelope["ok"], serde_json::json!(false));
    // Reporting three entries here would describe the conflicted file as if it
    // were publishable content.
    assert_eq!(
        envelope["error"]["details"]["entry_count"],
        serde_json::json!(0)
    );
}

#[test]
fn check_reports_conflict_markers_in_the_index() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        "# Development log\n\n## Months\n\n<<<<<<< HEAD\n- [2026-04](2026-04.md)\n=======\n- [2026-05](2026-05.md)\n>>>>>>> feature\n",
    )
    .expect("write conflicted index");

    let output = run_in(&fixture.root, &["check"]);
    let stdout = output.stdout_text();
    assert_eq!(output.code, 65, "stdout={stdout}");
    assert!(stdout.contains("conflict-markers"), "stdout={stdout}");
}

#[test]
fn new_refuses_to_write_into_a_conflicted_month_file() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        CONFLICTED_MONTH,
    )
    .expect("write conflicted month");

    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "Later entry",
            "--date",
            "2026-04-25",
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
    assert_eq!(output.code, 65, "stderr={}", output.stderr_text());
    assert!(
        output.stderr_text().contains("merge conflict"),
        "stderr={}",
        output.stderr_text()
    );
    // The refusal has to leave the file exactly as the merge left it, so the
    // author resolves one conflict rather than a conflict plus a new entry.
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), CONFLICTED_MONTH);
}

#[test]
fn new_reports_the_conflict_error_code_in_the_envelope() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        CONFLICTED_MONTH,
    )
    .expect("write conflicted month");

    let output = run_in(
        &fixture.root,
        &[
            "--format",
            "json",
            "new",
            "--title",
            "Later entry",
            "--date",
            "2026-04-25",
            "--result",
            "Shipped it.",
        ],
    );
    let envelope: serde_json::Value =
        serde_json::from_str(&output.stdout_text()).expect("parse envelope");
    assert_eq!(envelope["ok"], serde_json::json!(false));
    assert_eq!(
        envelope["error"]["code"],
        serde_json::json!("conflict-markers")
    );
}

#[test]
fn index_refuses_to_rewrite_a_conflicted_index() {
    let fixture = Fixture::new("docs/devlog");
    let conflicted = "# Development log\n\n## Months\n\n<<<<<<< HEAD\n- [2026-04](2026-04.md)\n=======\n- [2026-05](2026-05.md)\n>>>>>>> feature\n";
    std::fs::write(fixture.devlog_path("docs/devlog/README.md"), conflicted)
        .expect("write conflicted index");

    let output = run_in(&fixture.root, &["index"]);
    assert_eq!(output.code, 65, "stderr={}", output.stderr_text());
    // Rewriting the `## Months` section would resolve half of this conflict
    // and leave the markers around it in place.
    assert_eq!(fixture.read("docs/devlog/README.md"), conflicted);
}

const CONFLICTED_INDEX: &str = "# Development log\n\n## Months\n\n<<<<<<< HEAD\n- [2026-04](2026-04.md)\n=======\n- [2026-05](2026-05.md)\n>>>>>>> feature\n";

#[test]
fn new_refuses_before_writing_when_only_the_index_is_conflicted() {
    let fixture = Fixture::new("docs/devlog");
    let original = fixture.read("docs/devlog/2026-04.md");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        CONFLICTED_INDEX,
    )
    .expect("write conflicted index");

    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "Later entry",
            "--date",
            "2026-04-25",
            "--result",
            "Shipped it.",
        ],
    );
    assert_eq!(output.code, 65, "stderr={}", output.stderr_text());
    // `new` writes the month file and then the index. Checking only the index
    // write would leave the entry on disk behind a message saying nothing was
    // written, and a retry after resolving the conflict would insert it twice.
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), original);
}

#[test]
fn new_refuses_before_creating_a_month_file_when_the_index_is_conflicted() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/README.md"),
        CONFLICTED_INDEX,
    )
    .expect("write conflicted index");

    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "First of the month",
            "--date",
            "2026-06-02",
            "--result",
            "Shipped it.",
        ],
    );
    assert_eq!(output.code, 65, "stderr={}", output.stderr_text());
    assert!(
        !fixture.devlog_path("docs/devlog/2026-06.md").exists(),
        "the month file must not be created when the index is unusable"
    );
}

#[test]
fn an_entry_quoting_conflict_markers_in_a_fence_stays_writable() {
    // The entry documenting conflict handling is the obvious case: it quotes
    // the markers. Treating a quoted marker as a real one would make the month
    // permanently unwritable until someone edited the prose.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - Conflict handling\n\n### Result\n\n- The CLI now refuses a file holding markers such as:\n\n  ```text\n  <<<<<<< HEAD\n  =======\n  >>>>>>> feature\n  ```\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n### Links\n\n- d\n",
    )
    .expect("write entry quoting markers");

    let check = run_in(&fixture.root, &["check"]);
    assert_eq!(check.code, 0, "stdout={}", check.stdout_text());

    let new = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "Later entry",
            "--date",
            "2026-04-25",
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
    assert_eq!(new.code, 0, "stderr={}", new.stderr_text());
}

#[test]
fn a_month_file_whose_prose_contains_a_setext_underline_is_writable() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - Underlines\n\n### Result\n\n- A separator line follows.\n\n### Why / context\n\n- b\n\n### Evidence\n\n- =======\n\n### Links\n\n- d\n",
    )
    .expect("write entry containing a separator");

    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
}

#[test]
fn an_entry_without_links_or_follow_ups_is_complete() {
    // Measured across the logs that already existed: 60 of 483 hand-written
    // entries carry no Links section, because the author had nothing worth
    // linking. Requiring it would report those as defects.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - No links to keep\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n",
    )
    .expect("write entry without links");

    let output = run_in(&fixture.root, &["check"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
}

#[test]
fn each_required_section_is_still_reported_when_absent() {
    for (absent, body) in [
        (
            "Result",
            "### Why / context\n\n- b\n\n### Evidence\n\n- c\n",
        ),
        (
            "Why / context",
            "### Result\n\n- a\n\n### Evidence\n\n- c\n",
        ),
        (
            "Evidence",
            "### Result\n\n- a\n\n### Why / context\n\n- b\n",
        ),
    ] {
        let fixture = Fixture::new("docs/devlog");
        std::fs::write(
            fixture.devlog_path("docs/devlog/2026-04.md"),
            format!("# Development log - 2026-04\n\n## 2026-04-17 - Thin entry\n\n{body}"),
        )
        .expect("write thin entry");

        let output = run_in(&fixture.root, &["check"]);
        let stdout = output.stdout_text();
        assert_eq!(output.code, 65, "absent={absent} stdout={stdout}");
        assert!(
            stdout.contains(&format!("has no '### {absent}' section")),
            "absent={absent} stdout={stdout}"
        );
    }
}

#[test]
fn new_omits_an_optional_section_rather_than_writing_a_placeholder() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "No links",
            "--date",
            "2026-06-20",
            "--why",
            "It was needed.",
            "--evidence",
            "Ran the gate.",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    // A month with no existing file, so the assertions below see only the
    // entry this command wrote.
    let contents = fixture.read("docs/devlog/2026-06.md");
    assert!(!contents.contains("### Links"), "contents={contents}");
    assert!(!contents.contains("### Follow-ups"), "contents={contents}");
    // `--result` is deliberately absent: a required section the author left
    // empty still renders, and still renders its prompt, so the gap stays
    // visible instead of disappearing with the optional sections.
    assert!(
        contents.contains("### Result\n\n- TODO\n"),
        "contents={contents}"
    );

    let check = run_in(&fixture.root, &["check"]);
    assert_eq!(check.code, 0, "stdout={}", check.stdout_text());
}

#[test]
fn new_writes_a_link_the_workspace_markdown_lint_accepts() {
    let fixture = Fixture::new("docs/devlog");
    let output = run_in(
        &fixture.root,
        &[
            "new",
            "--title",
            "Linked entry",
            "--date",
            "2026-06-20",
            "--result",
            "Shipped it.",
            "--why",
            "It was needed.",
            "--evidence",
            "Ran the gate.",
            "--link",
            "https://github.com/sympoies/nils-cli/pull/1729",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let contents = fixture.read("docs/devlog/2026-06.md");
    assert!(
        contents.contains("- <https://github.com/sympoies/nils-cli/pull/1729>\n"),
        "contents={contents}"
    );

    let check = run_in(&fixture.root, &["check"]);
    assert_eq!(check.code, 0, "stdout={}", check.stdout_text());
}

#[test]
fn a_log_outside_the_repository_root_is_reported_with_one_leading_slash() {
    // `--dir` pointing at another checkout's log is the ordinary way to check
    // a log this repository does not own, so the path below is the supported
    // route rather than an edge case. POSIX leaves a leading `//`
    // implementation-defined, so a doubled slash is not merely cosmetic.
    let logged = Fixture::new("docs/devlog");
    let elsewhere = Fixture::new("docs/devlog");
    let absolute = logged.devlog_path("docs/devlog");
    let absolute = absolute.to_str().expect("utf-8 fixture path");

    let text = run_in(&elsewhere.root, &["--dir", absolute, "check"]);
    assert_eq!(text.code, 0, "stderr={}", text.stderr_text());
    assert!(
        text.stdout_text().starts_with(&format!("{absolute}:")),
        "stdout={}",
        text.stdout_text()
    );

    let json = run_in(
        &elsewhere.root,
        &["--format", "json", "--dir", absolute, "check"],
    );
    assert_eq!(json.code, 0, "stderr={}", json.stderr_text());
    assert_eq!(json.stdout_json()["data"]["devlog_dir"], absolute);
}

#[test]
fn a_problem_outside_the_repository_root_names_a_pastable_path() {
    let logged = Fixture::new("docs/devlog");
    let elsewhere = Fixture::new("docs/devlog");
    std::fs::write(
        logged.devlog_path("docs/devlog/2026-13.md"),
        "not a month\n",
    )
    .expect("write mis-named month file");
    let absolute = logged.devlog_path("docs/devlog");
    let absolute = absolute.to_str().expect("utf-8 fixture path");

    let json = run_in(
        &elsewhere.root,
        &["--format", "json", "--dir", absolute, "check"],
    );
    assert_eq!(json.code, 65, "stderr={}", json.stderr_text());
    // A failing check reports through the error envelope, so the problem rows
    // live under `error.details` rather than `data`.
    let reported = json.stdout_json()["error"]["details"]["problems"][0]["path"]
        .as_str()
        .expect("problem path")
        .to_string();
    assert_eq!(reported, format!("{absolute}/2026-13.md"));
    assert!(
        std::path::Path::new(&reported).is_file(),
        "reported path is not readable back: {reported}"
    );
}

#[test]
fn a_log_under_the_repository_root_is_reported_relative_with_no_leading_slash() {
    // The ordinary case, and the one the out-of-root assertions above do not
    // pin: a rendering that prefixed every in-repository path with a slash
    // would satisfy all of them.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-13.md"),
        "not a month\n",
    )
    .expect("write mis-named month file");

    let json = run_in(&fixture.root, &["--format", "json", "check"]);
    assert_eq!(json.code, 65, "stderr={}", json.stderr_text());
    let payload = json.stdout_json();
    assert_eq!(payload["error"]["details"]["devlog_dir"], "docs/devlog");
    assert_eq!(
        payload["error"]["details"]["problems"][0]["path"],
        "docs/devlog/2026-13.md"
    );
}
/// A month file in the shape every log in this organization was written in
/// before the CLI existed: bold labels instead of headings, and an em dash
/// where the parser splits on a hyphen.
const HAND_WRITTEN_MONTH: &str = "# Development log — 2026-04\n\n## 2026-04-17 — Existing entry\n\n**Result**\n\n- Did a thing.\n\n**Why / context**\n\n- Because.\n\n**Evidence**\n\n- Ran it.\n\n**Links**\n\n- `abc12345`\n";

#[test]
fn fix_rewrites_a_hand_written_month_into_the_shape_check_accepts() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        HAND_WRITTEN_MONTH,
    )
    .expect("write hand-written month");

    let before = run_in(&fixture.root, &["check"]);
    assert_eq!(before.code, 65, "the fixture must start broken");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    let contents = fixture.read("docs/devlog/2026-04.md");
    assert!(contents.starts_with("# Development log - 2026-04\n"));
    assert!(contents.contains("## 2026-04-17 - Existing entry\n"));
    assert!(contents.contains("### Result\n"));
    assert!(!contents.contains("**Result**"));
    // The bullet the author wrote is untouched; only the labels around it move.
    assert!(contents.contains("- Did a thing.\n"));

    let after = run_in(&fixture.root, &["check"]);
    assert_eq!(after.code, 0, "stdout={}", after.stdout_text());
}

#[test]
fn fix_does_not_promote_bold_prose_that_is_not_a_section_label() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - Entry\n\n### Result\n\n- Did a thing.\n\n**Not a section**\n\n**Why**\n\n**Follow-up**\n\n- The **Result** section was added later.\n\n  **Evidence**\n\n### Why / context\n\n- Because.\n\n### Evidence\n\n- Ran it.\n",
    )
    .expect("write month with bold prose");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    let contents = fixture.read("docs/devlog/2026-04.md");
    // Promoting this would invent a section the author never wrote, and
    // `check` would then report it as unknown.
    // Each of these would be taken by a looser rule, and each would invent a
    // section its author never wrote. `**Why**` and `**Follow-up**` are the
    // near misses a prefix match takes; the embedded span is what a substring
    // match takes; the indented one is inside a list item.
    for survivor in [
        "**Not a section**",
        "**Why**",
        "**Follow-up**",
        "- The **Result** section was added later.",
        "  **Evidence**",
    ] {
        assert!(
            contents.contains(survivor),
            "{survivor} was rewritten: contents={contents}"
        );
    }
}

#[test]
fn fix_keeps_a_dash_that_is_part_of_a_title() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - Split the read — write boundary\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n",
    )
    .expect("write month with a dash in the title");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    let contents = fixture.read("docs/devlog/2026-04.md");
    assert!(
        contents.contains("## 2026-04-17 - Split the read — write boundary\n"),
        "contents={contents}"
    );
}

#[test]
fn fix_repairs_a_month_heading_that_disagrees_with_its_filename() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-03\n\n## 2026-04-17 - Entry\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n",
    )
    .expect("write month with a wrong heading");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    // The filename is what search, the index and check are keyed on, so the
    // heading is the half that moves.
    let contents = fixture.read("docs/devlog/2026-04.md");
    assert!(contents.starts_with("# Development log - 2026-04\n"));
}

#[test]
fn fix_restores_newest_first_order_and_keeps_same_day_entries_as_written() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-01 - Oldest\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n## 2026-04-20 - First of the twentieth\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n## 2026-04-20 - Second of the twentieth\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n",
    )
    .expect("write out-of-order month");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    let contents = fixture.read("docs/devlog/2026-04.md");
    let first = contents
        .find("First of the twentieth")
        .expect("first entry");
    let second = contents
        .find("Second of the twentieth")
        .expect("second entry");
    let oldest = contents.find("Oldest").expect("oldest entry");
    assert!(first < oldest && second < oldest, "contents={contents}");
    // Two entries share a date, and that order carries the within-day sequence
    // the date alone cannot, so the sort must not disturb it.
    assert!(first < second, "contents={contents}");

    let after = run_in(&fixture.root, &["check"]);
    assert_eq!(after.code, 0, "stdout={}", after.stdout_text());
}

#[test]
fn fix_backfills_a_required_section_and_says_it_was_not_recorded() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - Entry\n\n### Result\n\n- Did a thing.\n\n### Links\n\n- `abc12345`\n",
    )
    .expect("write month missing two required sections");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    let contents = fixture.read("docs/devlog/2026-04.md");
    // Template order, not append order: the two added sections belong between
    // the Result that was written and the Links that were.
    let result = contents.find("### Result").expect("result");
    let why = contents.find("### Why / context").expect("why");
    let evidence = contents.find("### Evidence").expect("evidence");
    let links = contents.find("### Links").expect("links");
    assert!(
        result < why && why < evidence && evidence < links,
        "contents={contents}"
    );
    // The bullet records the absence rather than describing work nobody wrote.
    assert!(
        contents.contains("- Not recorded; added by `devlog fix`.\n"),
        "contents={contents}"
    );

    let after = run_in(&fixture.root, &["check"]);
    assert_eq!(after.code, 0, "stdout={}", after.stdout_text());
}

#[test]
fn fix_leaves_a_clean_log_byte_identical() {
    let fixture = Fixture::new("docs/devlog");
    let before = fixture.read("docs/devlog/2026-04.md");
    let index_before = fixture.read("docs/devlog/README.md");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    assert_eq!(fixture.read("docs/devlog/2026-04.md"), before);
    assert_eq!(fixture.read("docs/devlog/README.md"), index_before);
}

#[test]
fn fix_is_idempotent() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        HAND_WRITTEN_MONTH,
    )
    .expect("write hand-written month");

    let first = run_in(&fixture.root, &["fix"]);
    assert_eq!(first.code, 0, "stdout={}", first.stdout_text());
    let once = fixture.read("docs/devlog/2026-04.md");

    let second = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(second.code, 0, "stderr={}", second.stderr_text());
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), once);
    assert_eq!(second.stdout_json()["data"]["files_changed"], 0);
}

#[test]
fn fix_reports_what_it_cannot_repair_and_leaves_it_alone() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(fixture.devlog_path("docs/devlog/april.md"), "# nope\n")
        .expect("write mis-named file");

    // A file that is not a month has no single correct repair: renaming it
    // would guess at the month, deleting it would lose the entry.
    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert!(output.stdout_text().contains("unexpected-file"));
    assert_eq!(fixture.read("docs/devlog/april.md"), "# nope\n");
}

#[test]
fn fix_refuses_a_conflicted_log_and_writes_nothing() {
    let fixture = Fixture::new("docs/devlog");
    // The conflicted month sorts *after* the repairable one deliberately. With
    // the order reversed a per-file check refuses before ever reaching the file
    // it would have half-repaired, and the test cannot tell the two designs
    // apart.
    let conflicted = CONFLICTED_MONTH.replace("2026-04", "2026-05");
    std::fs::write(fixture.devlog_path("docs/devlog/2026-05.md"), &conflicted)
        .expect("write conflicted month");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        HAND_WRITTEN_MONTH,
    )
    .expect("write a repairable month before it");
    let repairable = fixture.read("docs/devlog/2026-04.md");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(fixture.read("docs/devlog/2026-05.md"), conflicted);
    // Every file is checked before any is written, so a repairable month
    // beside a conflicted one is not half-repaired behind a refusal.
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), repairable);
}

#[test]
fn fix_links_a_month_the_index_had_lost() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-05.md"),
        "# Development log - 2026-05\n\n## 2026-05-02 - Entry\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n",
    )
    .expect("write an unlisted month");

    let output = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert_eq!(output.stdout_json()["data"]["index_updated"], true);
    assert!(
        fixture
            .read("docs/devlog/README.md")
            .contains("- [2026-05](2026-05.md)")
    );
}

#[test]
fn fix_emits_a_json_envelope() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        HAND_WRITTEN_MONTH,
    )
    .expect("write hand-written month");

    let output = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let json = output.stdout_json();
    assert_eq!(json["schema_version"], "cli.devlog.fix.v1");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["files_changed"], 1);
    assert_eq!(json["data"]["repairs"]["section_labels"], 4);
    assert_eq!(json["data"]["repairs"]["heading_separators"], 2);
    assert_eq!(
        json["data"]["remaining"]
            .as_array()
            .expect("remaining")
            .len(),
        0
    );
}

#[test]
fn fix_reports_a_failure_envelope_when_something_survives_it() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(fixture.devlog_path("docs/devlog/april.md"), "# nope\n")
        .expect("write mis-named file");

    let output = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(output.code, 65);
    let json = output.stdout_json();
    assert_eq!(json["schema_version"], "cli.devlog.fix.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "structural-problems");
}

#[test]
fn fix_does_not_create_an_index_a_log_never_had() {
    let fixture = Fixture::new("docs/devlog");
    std::fs::remove_file(fixture.devlog_path("docs/devlog/README.md")).expect("remove index");

    // The index is also the log's conventions document, so writing one would
    // be authoring content rather than repairing structure.
    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert!(output.stdout_text().contains("missing-index"));
    assert!(!fixture.devlog_path("docs/devlog/README.md").exists());
}

#[test]
fn fix_leaves_an_entry_that_quotes_the_format_inside_a_fence_alone() {
    // An entry documenting this format is exactly the case `check` already
    // excludes fenced conflict markers for. Promoting the label in the example
    // would edit the example rather than the entry.
    let fixture = Fixture::new("docs/devlog");
    let quoting = "# Development log - 2026-04\n\n## 2026-04-17 - How an entry is shaped\n\n### Result\n\n- The old form looked like this:\n\n```markdown\n## 2026-04-17 — Title\n\n**Result**\n\n- what happened\n```\n\n### Why / context\n\n- Because.\n\n### Evidence\n\n- Ran it.\n";
    std::fs::write(fixture.devlog_path("docs/devlog/2026-04.md"), quoting)
        .expect("write month quoting the format");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), quoting);
}

#[test]
fn check_does_not_read_a_quoted_entry_heading_as_an_entry() {
    // The counterpart to the `fix` case above. If `check` counted the example
    // it would report a malformed heading and three missing sections in a
    // healthy log, and `fix` could repair none of them without editing prose.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-04.md"),
        "# Development log - 2026-04\n\n## 2026-04-17 - How an entry is shaped\n\n### Result\n\n- The old form looked like this:\n\n```markdown\n## 2026-04-01 — Title\n\n**Result**\n\n- what happened\n```\n\n### Why / context\n\n- Because.\n\n### Evidence\n\n- Ran it.\n",
    )
    .expect("write month quoting the format");

    let output = run_in(&fixture.root, &["--format", "json", "check"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    // One entry, not two: the quoted heading is an example.
    assert_eq!(output.stdout_json()["data"]["entry_count"], 1);
}

#[test]
fn fix_leaves_an_entry_whose_date_cannot_be_read_unsorted() {
    // Sorting around a heading whose date is unreadable would be guessing at
    // where that entry belongs. `check` reports the heading instead.
    let fixture = Fixture::new("docs/devlog");
    let unreadable = "# Development log - 2026-04\n\n## 2026-04-01 - Oldest\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n## Undated entry\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n## 2026-04-20 - Newest\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n";
    std::fs::write(fixture.devlog_path("docs/devlog/2026-04.md"), unreadable)
        .expect("write month with an unreadable entry date");

    let output = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), unreadable);
    let kinds: Vec<String> = output.stdout_json()["error"]["details"]["remaining"]
        .as_array()
        .expect("remaining")
        .iter()
        .map(|problem| problem["kind"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        kinds.iter().any(|kind| kind == "malformed-entry-heading"),
        "kinds={kinds:?}"
    );
}

#[test]
fn fix_appending_a_section_to_the_last_entry_leaves_one_trailing_newline() {
    // The only entry, with nothing after the sections it is missing, so both
    // insertions append at the tail rather than above a later section. That is
    // the branch that ends the file, and the one that previously left a blank
    // line behind it.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-06.md"),
        "# Development log - 2026-06\n\n## 2026-06-20 - Entry\n\n### Result\n\n- Did a thing.\n",
    )
    .expect("write a month whose last entry is missing its tail sections");

    let output = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert_eq!(
        output.stdout_json()["data"]["repairs"]["backfilled_sections"],
        2
    );

    let contents = fixture.read("docs/devlog/2026-06.md");
    assert!(contents.ends_with("fix`.\n"), "contents={contents}");
    assert!(!contents.contains("\n\n\n"), "contents={contents}");
}

#[test]
fn fix_leaves_a_month_file_that_does_not_open_with_a_heading_alone() {
    // Overwriting line one would lose whatever the author put there. The
    // heading repair only moves a heading that is already a heading.
    let fixture = Fixture::new("docs/devlog");
    let prose_first = "Some prose the author left at the top.\n\n## 2026-04-17 - Entry\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n";
    std::fs::write(fixture.devlog_path("docs/devlog/2026-04.md"), prose_first)
        .expect("write a month that does not open with a heading");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), prose_first);
    assert!(output.stdout_text().contains("missing-heading"));
}

#[test]
fn fix_reorders_without_losing_the_blank_line_before_a_heading() {
    // Two entries, in the wrong order, nothing else wrong — the most ordinary
    // case there is, and the one where the block that was last in the file
    // carries no trailing blank of its own.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-10.md"),
        "# Development log - 2026-10\n\n## 2026-10-01 - Old\n\n### Result\n\n- a\n\n### Why / context\n\n- b\n\n### Evidence\n\n- c\n\n## 2026-10-09 - New\n\n### Result\n\n- d\n\n### Why / context\n\n- e\n\n### Evidence\n\n- f\n",
    )
    .expect("write out-of-order month");

    let output = run_in(&fixture.root, &["--format", "json", "fix"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert_eq!(
        output.stdout_json()["data"]["repairs"]["reordered_entries"],
        2
    );

    let contents = fixture.read("docs/devlog/2026-10.md");
    let lines: Vec<&str> = contents.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 && line.starts_with("## ") {
            assert!(
                lines[index - 1].trim().is_empty(),
                "no blank line before {line}: contents={contents}"
            );
        }
    }
    assert!(contents.ends_with("- c\n"), "contents={contents}");
    assert!(!contents.contains("\n\n\n"), "contents={contents}");
}

#[test]
fn fix_backfills_around_a_fenced_example_without_touching_it() {
    // The entry quotes a `### Evidence` heading as an example and is missing a
    // real one. A fence-blind insertion counts the quoted heading as present
    // and uses it as the insertion point, writing the generated section into
    // the middle of the author's code block.
    let fixture = Fixture::new("docs/devlog");
    std::fs::write(
        fixture.devlog_path("docs/devlog/2026-06.md"),
        "# Development log - 2026-06\n\n## 2026-06-20 - How an entry is shaped\n\n### Result\n\n- The template looks like this:\n\n```markdown\n### Evidence\n\n- commands run\n```\n",
    )
    .expect("write a month quoting a section heading");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());

    let contents = fixture.read("docs/devlog/2026-06.md");
    // The example is byte-identical, and both real sections landed after the
    // whole fenced block rather than inside it.
    assert!(
        contents.contains("```markdown\n### Evidence\n\n- commands run\n```\n"),
        "contents={contents}"
    );
    let fence_end = contents.rfind("```").expect("closing fence");
    for section in ["### Why / context", "### Evidence\n\n- Not recorded"] {
        let at = contents.rfind(section).expect(section);
        assert!(
            at > fence_end,
            "{section} landed inside the fence: {contents}"
        );
    }

    let check = run_in(&fixture.root, &["check"]);
    assert_eq!(check.code, 0, "stdout={}", check.stdout_text());
}

#[test]
fn fix_leaves_a_crlf_month_file_alone_when_it_needs_nothing() {
    // `lines()` drops the carriage return, so rebuilding from it rewrites every
    // line of a CRLF file — a whole-file diff from a command that reported it
    // changed nothing.
    let fixture = Fixture::new("docs/devlog");
    let clean = fixture.read("docs/devlog/2026-04.md").replace('\n', "\r\n");
    std::fs::write(fixture.devlog_path("docs/devlog/2026-04.md"), &clean)
        .expect("write a CRLF month file");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(fixture.read("docs/devlog/2026-04.md"), clean);
    assert!(output.stdout_text().contains("nothing to repair"));
}

#[test]
fn fix_does_not_write_through_a_symlinked_month_file() {
    // A committed `2026-05.md -> ../../elsewhere` would let a repair land
    // outside the log while the link itself looks untouched in review.
    let fixture = Fixture::new("docs/devlog");
    let outside = fixture.devlog_path("outside.md");
    let before = "# Development log - 2026-05\n\n## 2026-05-02 - Entry\n\n**Result**\n\n- a\n";
    std::fs::write(&outside, before).expect("write the link target");
    std::os::unix::fs::symlink(&outside, fixture.devlog_path("docs/devlog/2026-05.md"))
        .expect("create the symlinked month file");

    let output = run_in(&fixture.root, &["fix"]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(
        std::fs::read_to_string(&outside).expect("read the link target"),
        before
    );
    assert!(output.stdout_text().contains("unexpected-file"));
}
