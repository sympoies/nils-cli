//! Integration coverage for `issue tracker lint | graph | tick`.
//!
//! The provider paths run the real binary against the file-backed
//! `--provider local` store, so every read and write goes through the same
//! `issue view` / `issue edit` / `issue comment` calls the GitHub and GitLab
//! backends use. `--body-file` paths run with a backend stub that fails when
//! invoked, proving a local draft needs no provider.

use std::fs;
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli, run_forge_cli_with_stdin};

const DATA: i32 = 65;
const USAGE: i32 = 64;
const TRACKING: &str = "workflow::tracking";

const NEVER_RUN: &str = "#!/bin/sh\necho 'a draft body must not reach a backend' >&2\nexit 99\n";

const TABLE: &str = "\
Program tracker.

## Phase table

### Phase 1

- [ ] **A1** First: #2
- [ ] **A2** Second: #3 · after A1
- [ ] **REL** Release · after A2
";

const GRAPH: &str = "graph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL";

/// `TABLE` followed by its current dependency graph section.
fn clean_body() -> String {
    format!("{TABLE}\n## Dependency graph\n\n```mermaid\n{GRAPH}\n```\n")
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tracker-row-grammar")
        .join(name);
    String::from_utf8(fs::read(path).expect("read fixture")).expect("utf-8 fixture")
}

fn fixture_at(dir: &str, name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(dir)
        .join(name);
    String::from_utf8(fs::read(path).expect("read fixture")).expect("utf-8 fixture")
}

fn fixture_path(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tracker-row-grammar")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// A hermetic local store plus a scratch directory for body files.
struct Forge {
    stub: StubEnv,
}

impl Forge {
    fn new() -> Self {
        Self {
            stub: StubEnv::new(),
        }
    }

    fn store(&self) -> PathBuf {
        self.stub.tempdir.path().join("store")
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.stub.tempdir.path().join(name);
        fs::write(&path, text).expect("write file");
        path.to_string_lossy().into_owned()
    }

    /// Run `forge-cli --provider local … --format json <args>`.
    fn run(&self, args: &[&str]) -> CmdOutput {
        self.run_as("json", args)
    }

    fn run_as(&self, format: &str, args: &[&str]) -> CmdOutput {
        let store = self.store().to_string_lossy().into_owned();
        let mut full: Vec<&str> = vec![
            "--provider",
            "local",
            "--store-root",
            &store,
            "--repo",
            "local:demo",
            "--format",
            format,
        ];
        full.extend_from_slice(args);
        run_forge_cli(&self.stub, &full)
    }

    fn ok(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        assert_eq!(
            out.code, 0,
            "{args:?}: stdout={} stderr={}",
            out.stdout, out.stderr
        );
        parse_envelope(&out.stdout)
    }

    /// Create an issue and return its number.
    fn create(&self, body: &str, labels: &[&str]) -> u64 {
        let body_file = self.file("create-body.md", body);
        let mut args = vec![
            "issue",
            "create",
            "--title",
            "Tracker",
            "--body-file",
            &body_file,
        ];
        for label in labels {
            args.extend_from_slice(&["--label", label]);
        }
        self.ok(&args)["data"]["number"].as_u64().expect("number")
    }

    fn body(&self, id: u64) -> String {
        let id = id.to_string();
        self.ok(&["issue", "view", &id])["data"]["body"]
            .as_str()
            .expect("body")
            .to_string()
    }

    fn comments(&self, id: u64) -> Vec<String> {
        let id = id.to_string();
        self.ok(&["issue", "view", &id, "--with-comments"])["data"]["comments"]
            .as_array()
            .expect("comments")
            .iter()
            .map(|comment| comment["body"].as_str().expect("comment body").to_string())
            .collect()
    }
}

fn findings(env: &Value) -> Vec<Value> {
    env["data"]["findings"]
        .as_array()
        .expect("data.findings")
        .iter()
        .map(|finding| {
            assert!(
                finding["message"].as_str().is_some_and(|m| !m.is_empty()),
                "finding without a message: {finding}"
            );
            json!({"code": finding["code"], "line": finding["line"], "ids": finding["ids"]})
        })
        .collect()
}

fn assert_findings_failure(out: &CmdOutput, schema: &str) -> Value {
    assert_eq!(
        out.code, DATA,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], schema);
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "tracker_findings");
    env
}

// ----- lint ------------------------------------------------------------------

#[test]
fn lint_reports_a_clean_tracker() {
    let forge = Forge::new();
    let id = forge.create(&clean_body(), &[TRACKING]);
    let env = forge.ok(&["issue", "tracker", "lint", &id.to_string()]);
    assert_eq!(env["schema_version"], "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(env["ok"], true);
    assert_eq!(
        env["data"],
        json!({
            "source": "issue",
            "provider": "local",
            "number": 1,
            "url": "local://demo/issues/1",
            "row_count": 3,
            "state_checked": false,
            "findings": [],
        })
    );
}

#[test]
fn lint_reports_grammar_findings_and_exits_data() {
    let forge = Forge::new();
    let body = "\
## Phase table

### Phase 1: Build

- [x] **A1** First: example/alpha#1
- [ ] **A2** Second: example/alpha#2 · after A1, A9
- [ ] **REL** Release · after A2, B7
";
    let id = forge.create(body, &[TRACKING]);
    let out = forge.run(&["issue", "tracker", "lint", &id.to_string()]);
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(
        findings(&env),
        [
            json!({"code": "unknown-dependency", "line": 6, "ids": ["A2", "A9"]}),
            json!({"code": "unknown-dependency", "line": 7, "ids": ["REL", "B7"]}),
        ]
    );
    assert_eq!(env["data"]["row_count"], 3);

    let text = forge.run_as("text", &["issue", "tracker", "lint", &id.to_string()]);
    assert_eq!(text.code, DATA);
    assert!(
        text.stdout.contains("line 6: unknown-dependency"),
        "{}",
        text.stdout
    );
    assert!(text.stderr.contains("tracker_findings"), "{}", text.stderr);
}

#[test]
fn lint_reports_a_missing_tracking_label() {
    let forge = Forge::new();
    let id = forge.create(&clean_body(), &["type::feature"]);
    let out = forge.run(&["issue", "tracker", "lint", &id.to_string()]);
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(
        findings(&env),
        [json!({"code": "missing-tracking-label", "line": null, "ids": []})]
    );
}

#[test]
fn lint_check_state_reports_mismatches_in_both_directions() {
    let forge = Forge::new();
    let table = "\
## Phase table

- [x] **D1** Done but its issue is open: #2
- [ ] **O1** Open but its issue is closed: #3
- [x] **D2** Done and closed: #4
- [ ] **O2** Open and open: #2
- [x] **U1** Its issue does not exist: #9
- [ ] **U2** Another repository is out of the store's reach: other/repo#2
- [ ] **G1** A gate has no issue

## Dependency graph

```mermaid
graph LR
  D1
  O1
  D2
  O2
  U1
  U2
  G1{{G1}}
```
";
    let tracker = forge.create(table, &[TRACKING]);
    assert_eq!(tracker, 1);
    for _ in 2..=4 {
        forge.create("child", &[]);
    }
    forge.ok(&["issue", "close", "3"]);
    forge.ok(&["issue", "close", "4"]);

    // Without --check-state no referenced issue is read: the tracker is clean.
    let env = forge.ok(&["issue", "tracker", "lint", "1"]);
    assert_eq!(findings(&env), Vec::<Value>::new());

    let out = forge.run(&["issue", "tracker", "lint", "1", "--check-state"]);
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(env["data"]["state_checked"], true);
    // D1 and O2 share issue #2 (open): D1 alone being done is one delivery
    // step, so neither row disagrees.
    assert_eq!(
        findings(&env),
        [
            json!({"code": "state-mismatch", "line": 4, "ids": ["O1"]}),
            json!({"code": "unreadable-ref", "line": 7, "ids": ["U1"]}),
            json!({"code": "unreadable-ref", "line": 8, "ids": ["U2"]}),
        ]
    );

    // The other direction: a tracker whose only row for open issue #2 is done.
    let done_open = "\
## Phase table

- [x] **D1** Done but its issue is open: #2

## Dependency graph

```mermaid
graph LR
  D1
```
";
    let second = forge.create(done_open, &[TRACKING]);
    let out = forge.run(&[
        "issue",
        "tracker",
        "lint",
        &second.to_string(),
        "--check-state",
    ]);
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(
        findings(&env),
        [json!({"code": "state-mismatch", "line": 3, "ids": ["D1"]})]
    );
}

#[test]
fn lint_body_file_needs_no_provider() {
    let stub = StubEnv::new().gh_stub(NEVER_RUN).glab_stub(NEVER_RUN);
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "lint",
            "--body-file",
            &fixture_path("valid/full.md"),
        ],
    );
    assert_eq!(out.code, 0, "stdout={} stderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["source"], "body-file");
    assert_eq!(env["data"]["provider"], Value::Null);
    assert_eq!(env["data"]["number"], Value::Null);
    assert_eq!(findings(&env), Vec::<Value>::new());

    // A draft is never checked for the tracking label, only for the grammar.
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "lint",
            "--body-file",
            &fixture_path("invalid/stale-graph--missing-section.md"),
        ],
    );
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(
        findings(&env),
        [json!({"code": "stale-graph", "line": null, "ids": []})]
    );
}

#[test]
fn lint_body_file_dash_reads_stdin() {
    let stub = StubEnv::new().gh_stub(NEVER_RUN);
    let out = run_forge_cli_with_stdin(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "lint",
            "--body-file",
            "-",
        ],
        &fixture("invalid/self-dependency.md"),
    );
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(
        findings(&env),
        [json!({"code": "self-dependency", "line": 4, "ids": ["A2"]})]
    );
}

#[test]
fn lint_rejects_an_id_or_check_state_together_with_body_file() {
    let stub = StubEnv::new().gh_stub(NEVER_RUN);
    let draft = fixture_path("valid/minimal.md");
    for args in [
        vec![
            "issue",
            "tracker",
            "lint",
            "--body-file",
            &draft,
            "--check-state",
        ],
        vec!["issue", "tracker", "lint", "1", "--body-file", &draft],
        vec!["issue", "tracker", "lint"],
        vec!["issue", "tracker", "graph"],
    ] {
        let mut full = vec!["--format", "json"];
        full.extend_from_slice(&args);
        let out = run_forge_cli(&stub, &full);
        assert_eq!(out.code, USAGE, "{args:?}: stdout={}", out.stdout);
    }
}

#[test]
fn read_only_dry_run_renders_the_view_plan_without_a_backend_call() {
    let stub = StubEnv::new().gh_stub(NEVER_RUN);
    for (command, schema) in [
        ("lint", "cli.forge-cli.issue.tracker.lint.v1"),
        ("graph", "cli.forge-cli.issue.tracker.graph.v1"),
        ("show", "cli.forge-cli.issue.tracker.show.v1"),
    ] {
        let out = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--dry-run",
                "--format",
                "json",
                "issue",
                "tracker",
                command,
                "7",
            ],
        );
        assert_eq!(out.code, 0, "{command}: stderr={}", out.stderr);
        let env = parse_envelope(&out.stdout);
        assert_eq!(env["schema_version"], schema);
        let plan: Vec<&str> = env["data"]["plan"]
            .as_array()
            .expect("plan")
            .iter()
            .map(|arg| arg.as_str().unwrap_or(""))
            .collect();
        assert_eq!(plan[1..4], ["issue", "view", "7"], "{command}");
    }
}

#[test]
fn show_ref_repository_replaces_the_repo_flag() {
    let stub = StubEnv::new().gh_stub(NEVER_RUN);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "flag/repo",
            "--dry-run",
            "--format",
            "json",
            "issue",
            "tracker",
            "show",
            "ref/repo#7",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let plan = parse_envelope(&out.stdout)["data"]["plan"].to_string();
    assert!(plan.contains("ref/repo"), "{plan}");
    assert!(!plan.contains("flag/repo"), "{plan}");
}

// ----- graph -----------------------------------------------------------------

#[test]
fn graph_prints_the_generated_block() {
    let forge = Forge::new();
    let id = forge.create(TABLE, &[TRACKING]).to_string();
    let env = forge.ok(&["issue", "tracker", "graph", &id]);
    assert_eq!(
        env["schema_version"],
        "cli.forge-cli.issue.tracker.graph.v1"
    );
    assert_eq!(
        env["data"],
        json!({
            "source": "issue",
            "provider": "local",
            "number": 1,
            "url": "local://demo/issues/1",
            "graph": GRAPH,
            "current": false,
            "change": "none",
            "changed": false,
            "written": false,
            "dry_run": false,
            "findings": [],
        })
    );
    assert_eq!(forge.body(1), TABLE);

    let text = forge.run_as("text", &["issue", "tracker", "graph", &id]);
    assert_eq!(text.code, 0, "stderr={}", text.stderr);
    assert_eq!(text.stdout, format!("```mermaid\n{GRAPH}\n```\n"));
}

#[test]
fn graph_refuses_a_table_with_row_findings() {
    let forge = Forge::new();
    let body = fixture("invalid/cycle.md");
    let id = forge.create(&body, &[TRACKING]).to_string();
    for args in [
        vec!["issue", "tracker", "graph", id.as_str()],
        vec!["issue", "tracker", "graph", id.as_str(), "--write"],
    ] {
        let out = forge.run(&args);
        let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.graph.v1");
        assert_eq!(env["data"]["graph"], Value::Null);
        assert_eq!(env["data"]["changed"], false);
        assert_eq!(env["data"]["written"], false);
        assert_eq!(
            findings(&env),
            [
                json!({"code": "cycle", "line": 3, "ids": ["A1", "A2", "A3"]}),
                json!({"code": "cycle", "line": 8, "ids": ["B2", "B3"]}),
            ]
        );
    }
    assert_eq!(forge.body(1), body);
}

#[test]
fn graph_write_inserts_the_section_and_a_second_run_changes_nothing() {
    let forge = Forge::new();
    let body = fixture("invalid/stale-graph--missing-section.md");
    let id = forge.create(&body, &[TRACKING]).to_string();

    let env = forge.ok(&["issue", "tracker", "graph", &id, "--write"]);
    assert_eq!(env["data"]["change"], "inserted-section");
    assert_eq!(env["data"]["changed"], true);
    assert_eq!(env["data"]["written"], true);
    assert_eq!(env["data"]["current"], false);
    assert!(env["data"].get("actions").is_none(), "{env}");
    let block = "## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n```\n\n";
    assert_eq!(
        forge.body(1),
        body.replace("## Open decisions", &format!("{block}## Open decisions"))
    );
    forge.ok(&["issue", "tracker", "lint", &id]);

    let written = forge.body(1);
    let env = forge.ok(&["issue", "tracker", "graph", &id, "--write"]);
    assert_eq!(env["data"]["change"], "none");
    assert_eq!(env["data"]["changed"], false);
    assert_eq!(env["data"]["written"], false);
    assert_eq!(env["data"]["current"], true);
    assert_eq!(forge.body(1), written);
}

#[test]
fn graph_write_replaces_a_stale_block_and_inserts_a_missing_one() {
    let forge = Forge::new();
    let stale = fixture("invalid/stale-graph--different.md");
    let id = forge.create(&stale, &[TRACKING]).to_string();
    let env = forge.ok(&["issue", "tracker", "graph", &id, "--write"]);
    assert_eq!(env["data"]["change"], "replaced-block");
    assert_eq!(
        forge.body(1),
        stale.replace(
            "  A1\n  A2\n  A1 --> A2\n",
            "  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n"
        )
    );

    let no_block = fixture("invalid/stale-graph--no-block.md");
    let id = forge.create(&no_block, &[TRACKING]).to_string();
    let env = forge.ok(&["issue", "tracker", "graph", &id, "--write"]);
    assert_eq!(env["data"]["change"], "inserted-block");
    assert_eq!(
        forge.body(2),
        no_block.replace(
            "## Dependency graph\n",
            "## Dependency graph\n\n```mermaid\ngraph LR\n  A1\n  A2\n  REL{{REL}}\n  A1 --> A2\n  A2 --> REL\n```\n"
        )
    );
}

#[test]
fn graph_write_dry_run_shows_the_plan_without_writing() {
    let forge = Forge::new();
    let id = forge.create(TABLE, &[TRACKING]).to_string();
    let env = forge.ok(&["--dry-run", "issue", "tracker", "graph", &id, "--write"]);
    assert_eq!(env["data"]["dry_run"], true);
    assert_eq!(env["data"]["change"], "inserted-section");
    assert_eq!(env["data"]["changed"], true);
    assert_eq!(env["data"]["written"], false);
    assert_eq!(env["data"]["actions"][0]["kind"], "edit-body");
    let plan = env["data"]["actions"][0]["plan"].as_array().expect("plan");
    assert!(plan.iter().any(|arg| *arg == clean_body()), "{plan:?}");
    assert_eq!(forge.body(1), TABLE);
}

#[test]
fn graph_body_file_writes_the_draft_and_never_writes_stdin() {
    let stub = StubEnv::new().gh_stub(NEVER_RUN);
    let draft = stub.tempdir.path().join("draft.md");
    fs::write(&draft, TABLE).expect("write draft");
    let draft = draft.to_string_lossy().into_owned();

    // Read-only by default.
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "graph",
            "--body-file",
            &draft,
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["source"], "body-file");
    assert_eq!(env["data"]["graph"], GRAPH);
    assert_eq!(fs::read_to_string(&draft).unwrap(), TABLE);

    // --dry-run reports the change and leaves the file alone.
    let out = run_forge_cli(
        &stub,
        &[
            "--dry-run",
            "--format",
            "json",
            "issue",
            "tracker",
            "graph",
            "--body-file",
            &draft,
            "--write",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["changed"], true);
    assert_eq!(env["data"]["written"], false);
    assert_eq!(fs::read_to_string(&draft).unwrap(), TABLE);

    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "graph",
            "--body-file",
            &draft,
            "--write",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["change"], "inserted-section");
    assert_eq!(env["data"]["written"], true);
    assert_eq!(fs::read_to_string(&draft).unwrap(), clean_body());

    let out = run_forge_cli_with_stdin(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "graph",
            "--body-file",
            "-",
            "--write",
        ],
        TABLE,
    );
    assert_eq!(out.code, DATA, "stdout={}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "tracker_stdin_not_writable"
    );
}

#[test]
fn graph_write_refuses_a_body_without_a_phase_table() {
    let forge = Forge::new();
    let unrelated = "An unrelated issue with no tracker sections.\n";
    let id = forge.create(unrelated, &[]).to_string();
    for args in [
        vec!["issue", "tracker", "graph", id.as_str(), "--write"],
        vec![
            "--dry-run",
            "issue",
            "tracker",
            "graph",
            id.as_str(),
            "--write",
        ],
    ] {
        let out = forge.run(&args);
        assert_eq!(out.code, DATA, "{args:?}: stdout={}", out.stdout);
        let env = parse_envelope(&out.stdout);
        assert_eq!(env["ok"], false);
        assert_eq!(env["error"]["code"], "tracker_no_phase_table");
    }
    assert_eq!(forge.body(1), unrelated);

    // Reading is still allowed and yields the zero-row graph.
    let env = forge.ok(&["issue", "tracker", "graph", &id]);
    assert_eq!(env["data"]["graph"], "graph LR");

    // The same guard protects a draft file.
    let draft = forge.file("unrelated.md", unrelated);
    let stub = StubEnv::new().gh_stub(NEVER_RUN);
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "issue",
            "tracker",
            "graph",
            "--body-file",
            &draft,
            "--write",
        ],
    );
    assert_eq!(out.code, DATA, "stdout={}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "tracker_no_phase_table"
    );
    assert_eq!(fs::read_to_string(&draft).unwrap(), unrelated);
}

#[test]
fn a_table_over_the_row_limit_is_refused_by_every_command() {
    let mut body = String::from("## Phase table\n\n");
    for n in 1..=501 {
        body.push_str(&format!("- [ ] **A{n}** Row {n}: #{n}\n"));
    }
    let forge = Forge::new();
    let id = forge.create(&body, &[TRACKING]).to_string();

    let out = forge.run(&["issue", "tracker", "lint", &id, "--check-state"]);
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.lint.v1");
    assert_eq!(
        findings(&env),
        [json!({"code": "too-many-rows", "line": null, "ids": []})]
    );

    let out = forge.run(&["issue", "tracker", "graph", &id, "--write"]);
    let env = assert_findings_failure(&out, "cli.forge-cli.issue.tracker.graph.v1");
    assert_eq!(
        findings(&env),
        [json!({"code": "too-many-rows", "line": null, "ids": []})]
    );

    let out = forge.run(&["issue", "tracker", "tick", &id, "--item", "A1"]);
    assert_eq!(out.code, DATA, "stdout={}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "tracker_too_many_rows"
    );
    assert_eq!(forge.body(1), body);
}

// ----- tick ------------------------------------------------------------------

#[test]
fn tick_marks_the_row_records_the_pr_and_posts_the_comment() {
    let forge = Forge::new();
    let body = clean_body();
    let id = forge.create(&body, &[TRACKING]).to_string();
    let comment = forge.file("comment.md", "Delivered A2 in #30.\n");

    let env = forge.ok(&[
        "issue",
        "tracker",
        "tick",
        &id,
        "--item",
        "A2",
        "--pr",
        "#30",
        "--comment-file",
        &comment,
    ]);
    assert_eq!(env["schema_version"], "cli.forge-cli.issue.tracker.tick.v1");
    assert_eq!(
        env["data"],
        json!({
            "provider": "local",
            "number": 1,
            "url": "local://demo/issues/1",
            "item": "A2",
            "line": 8,
            "row_before": "- [ ] **A2** Second: #3 · after A1",
            "row_after": "- [x] **A2** Second: #3 (PR #30) · after A1",
            "changed": true,
            "written": true,
            "dry_run": false,
            "comment_posted": true,
            "comment_url": "local://demo/issues/1#comment-1",
        })
    );
    assert_eq!(
        forge.body(1),
        body.replace(
            "- [ ] **A2** Second: #3 · after A1",
            "- [x] **A2** Second: #3 (PR #30) · after A1"
        )
    );
    assert_eq!(forge.comments(1), ["Delivered A2 in #30.\n"]);
    // The graph carries no done state, so the tracker is still clean.
    forge.ok(&["issue", "tracker", "lint", &id]);

    // Ticking again changes nothing and posts no second comment.
    let ticked = forge.body(1);
    let env = forge.ok(&[
        "issue",
        "tracker",
        "tick",
        &id,
        "--item",
        "A2",
        "--pr",
        "#30",
        "--comment-file",
        &comment,
    ]);
    assert_eq!(env["data"]["changed"], false);
    assert_eq!(env["data"]["written"], false);
    assert_eq!(env["data"]["comment_posted"], false);
    assert_eq!(env["data"]["comment_url"], Value::Null);
    assert_eq!(forge.body(1), ticked);
    assert_eq!(forge.comments(1).len(), 1);

    // A second PR is appended inside the same notes group.
    let env = forge.ok(&[
        "issue", "tracker", "tick", &id, "--item", "A2", "--pr", "#31",
    ]);
    assert_eq!(
        env["data"]["row_after"],
        "- [x] **A2** Second: #3 (PR #30, PR #31) · after A1"
    );
}

#[test]
fn tick_refuses_unknown_duplicated_and_malformed_items() {
    let forge = Forge::new();
    let body = "\
## Phase table

- [ ] **A1** First: #2
- [ ] **A1** Reuses the id: #3
- [ ] **A2** Malformed by its list: #4 · after A1 and more
";
    let id = forge.create(body, &[TRACKING]).to_string();
    for (item, code) in [
        ("A9", "tracker_item_unknown"),
        ("A1", "tracker_item_duplicated"),
        ("A2", "tracker_item_malformed"),
    ] {
        let out = forge.run(&["issue", "tracker", "tick", &id, "--item", item]);
        assert_eq!(out.code, DATA, "{item}: stdout={}", out.stdout);
        let env = parse_envelope(&out.stdout);
        assert_eq!(env["ok"], false, "{item}");
        assert_eq!(env["error"]["code"], code, "{item}");
    }
    let out = forge.run(&[
        "issue", "tracker", "tick", &id, "--item", "A1", "--pr", "a b",
    ]);
    assert_eq!(out.code, DATA);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "tracker_pr_invalid"
    );
    assert_eq!(forge.body(1), body);
}

#[test]
fn tick_dry_run_shows_the_row_without_writing() {
    let forge = Forge::new();
    let body = clean_body();
    let id = forge.create(&body, &[TRACKING]).to_string();
    let comment = forge.file("comment.md", "Delivered A1.\n");
    let env = forge.ok(&[
        "--dry-run",
        "issue",
        "tracker",
        "tick",
        &id,
        "--item",
        "A1",
        "--comment-file",
        &comment,
    ]);
    assert_eq!(env["data"]["dry_run"], true);
    assert_eq!(env["data"]["changed"], true);
    assert_eq!(env["data"]["written"], false);
    assert_eq!(env["data"]["comment_posted"], false);
    assert_eq!(env["data"]["row_after"], "- [x] **A1** First: #2");
    let kinds: Vec<&str> = env["data"]["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .map(|action| action["kind"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(kinds, ["edit-body", "comment"]);
    assert_eq!(forge.body(1), body);
    assert!(forge.comments(1).is_empty());
}

#[test]
fn tick_keeps_what_another_session_wrote_since_the_last_read() {
    let forge = Forge::new();
    let id = forge.create(&clean_body(), &[TRACKING]).to_string();
    forge.ok(&["issue", "tracker", "tick", &id, "--item", "A1"]);

    // Another session ticks REL and adds a note by editing the stored issue
    // directly, after this session last read the tracker.
    let record = forge.store().join("issues/1.json");
    let mut issue: Value = serde_json::from_str(&fs::read_to_string(&record).unwrap()).unwrap();
    let theirs = format!(
        "{}\nA note from another session.\n",
        issue["body"]
            .as_str()
            .unwrap()
            .replace("- [ ] **REL**", "- [x] **REL**")
    );
    issue["body"] = Value::String(theirs.clone());
    fs::write(&record, serde_json::to_string_pretty(&issue).unwrap()).unwrap();

    forge.ok(&["issue", "tracker", "tick", &id, "--item", "A2"]);
    assert_eq!(
        forge.body(1),
        theirs.replace("- [ ] **A2**", "- [x] **A2**")
    );
}

// ----- show ------------------------------------------------------------------

fn show_fixture(name: &str) -> (String, Vec<Value>) {
    let body = fixture_at("tracker-show", &format!("{name}.md"));
    let rows: Vec<Value> =
        serde_json::from_str(&fixture_at("tracker-show", &format!("{name}.rows.json")))
            .expect("rows fixture");
    (body, rows)
}

/// The rows a fixture expects once an own-repository `#N` is qualified with
/// the tracker's repository.
fn qualified(rows: &[Value], repo: &str) -> Vec<Value> {
    rows.iter()
        .map(|row| {
            let mut row = row.clone();
            if let Some(reference) = row["reference"].as_str()
                && reference.starts_with('#')
            {
                row["reference"] = json!(format!("{repo}{reference}"));
            }
            row
        })
        .collect()
}

impl Forge {
    fn create_titled(&self, title: &str, body: &str, labels: &[&str]) -> u64 {
        let body_file = self.file("create-body.md", body);
        let mut args = vec![
            "issue",
            "create",
            "--title",
            title,
            "--body-file",
            &body_file,
        ];
        for label in labels {
            args.extend_from_slice(&["--label", label]);
        }
        self.ok(&args)["data"]["number"].as_u64().expect("number")
    }
}

#[test]
fn show_serializes_the_rows_of_real_trackers() {
    for (name, title) in [
        (
            "dsh-runtime-kit-306",
            "Track agent-runtime-kit policy parity for DSH as primary harness",
        ),
        (
            "sympoies-infra-1072",
            "Track private-repo Actions migration to self-hosted runners",
        ),
    ] {
        let (body, rows) = show_fixture(name);
        let forge = Forge::new();
        let id = forge.create_titled(title, &body, &[TRACKING, "area::ci"]);
        let env = forge.ok(&["issue", "tracker", "show", &id.to_string()]);
        assert_eq!(env["schema_version"], "cli.forge-cli.issue.tracker.show.v1");
        assert_eq!(env["ok"], true, "{name}");
        assert_eq!(
            env["data"],
            json!({
                "source": "issue",
                "provider": "local",
                "number": 1,
                "url": "local://demo/issues/1",
                "repo": "local:demo",
                "title": title,
                "state": "open",
                "labels": [TRACKING, "area::ci"],
                "row_count": rows.len(),
                "rows": qualified(&rows, "local:demo"),
                "findings": [],
            }),
            "{name}"
        );
    }
}

#[test]
fn show_accepts_the_ref_forms_of_a_tracker() {
    let (body, rows) = show_fixture("dsh-runtime-kit-306");
    let forge = Forge::new();
    let id = forge
        .create_titled("Tracker", &body, &[TRACKING])
        .to_string();
    for reference in [id.clone(), format!("#{id}"), format!("local:demo#{id}")] {
        let env = forge.ok(&["issue", "tracker", "show", &reference]);
        assert_eq!(env["data"]["number"], 1, "{reference}");
        assert_eq!(
            env["data"]["rows"],
            json!(qualified(&rows, "local:demo")),
            "{reference}"
        );
    }
    let out = forge.run(&["issue", "tracker", "show", "not-a-ref"]);
    assert_eq!(
        out.code, USAGE,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
}

#[test]
fn show_keeps_valid_rows_and_reports_findings_without_failing() {
    // A cycle makes `lint` and `graph` refuse; the board still needs the lanes.
    let body = "\
## Phase table

- [x] **A1** First: #2 · after A2
- [ ] **A2** Second: other/repo#3 · after A1
- [ ] this line is not a row but starts like one
- [ ] **B1**missing space
";
    let forge = Forge::new();
    let id = forge.create_titled("Cyclic", body, &[TRACKING]);
    let env = forge.ok(&["issue", "tracker", "show", &id.to_string()]);
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["row_count"], 2);
    assert_eq!(
        env["data"]["rows"],
        json!([
            {"id": "A1", "title": "First", "reference": "local:demo#2", "done": true,
             "phase": null, "after": ["A2"], "notes": null, "line": 3},
            {"id": "A2", "title": "Second", "reference": "other/repo#3", "done": false,
             "phase": null, "after": ["A1"], "notes": null, "line": 4},
        ])
    );
    let codes: Vec<Value> = findings(&env).iter().map(|f| f["code"].clone()).collect();
    assert_eq!(
        codes,
        [
            json!("malformed-row"),
            json!("malformed-row"),
            json!("cycle")
        ]
    );
}

#[test]
fn show_reports_a_body_without_a_phase_table_as_zero_rows() {
    let forge = Forge::new();
    let id = forge.create_titled("Plain issue", "Nothing here.\n", &[]);
    let env = forge.ok(&["issue", "tracker", "show", &id.to_string()]);
    assert_eq!(env["data"]["rows"], json!([]));
    assert_eq!(env["data"]["row_count"], 0);
    assert_eq!(env["data"]["findings"], json!([]));
}

#[test]
fn show_reports_a_closed_tracker_and_does_not_check_the_graph_block() {
    let (body, _) = show_fixture("dsh-runtime-kit-306");
    let stale = body.replace("  N1 --> S1\n", "");
    let forge = Forge::new();
    let id = forge
        .create_titled("Tracker", &stale, &[TRACKING])
        .to_string();
    forge.ok(&["issue", "close", &id]);
    let env = forge.ok(&["issue", "tracker", "show", &id]);
    assert_eq!(env["data"]["state"], "closed");
    assert_eq!(env["data"]["findings"], json!([]));
}

#[test]
fn show_reports_a_missing_issue_as_a_backend_error() {
    let forge = Forge::new();
    let out = forge.run(&["issue", "tracker", "show", "99"]);
    assert_ne!(out.code, 0);
    assert_eq!(parse_envelope(&out.stdout)["ok"], false);
}

// ----- help ------------------------------------------------------------------

#[test]
fn help_lists_the_tracker_subcommands_and_their_flags() {
    let stub = StubEnv::new();
    let out = run_forge_cli(&stub, &["issue", "--help"]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("tracker"), "{}", out.stdout);

    let out = run_forge_cli(&stub, &["issue", "tracker", "--help"]);
    assert_eq!(out.code, 0);
    for sub in ["lint", "graph", "tick", "show"] {
        assert!(out.stdout.contains(sub), "missing {sub}: {}", out.stdout);
    }

    for (sub, flags) in [
        (
            "lint",
            vec!["--body-file", "--check-state", "tracker_findings"],
        ),
        (
            "graph",
            vec!["--body-file", "--write", "## Dependency graph"],
        ),
        (
            "tick",
            vec!["--item", "--pr", "--comment-file", "tracker_item_unknown"],
        ),
        (
            "show",
            vec!["REF", "owner/repo#N", "malformed-row", "too-many-rows"],
        ),
    ] {
        let out = run_forge_cli(&stub, &["issue", "tracker", sub, "--help"]);
        assert_eq!(out.code, 0, "{sub}");
        for flag in flags {
            assert!(
                out.stdout.contains(flag),
                "{sub} --help missing {flag}: {}",
                out.stdout
            );
        }
    }
}
