//! Shared pieces of the `issue tracker lint | graph | tick` atoms: the finding
//! payload, the tracker target, the single read every command starts from, and
//! the failure envelope that still carries the findings.
//!
//! The grammar itself lives in [`crate::tracker`] and is pure. These ops only
//! add IO: they read the tracker through the `issue view` call and write it
//! through the `issue edit` / `issue comment` calls, so every provider those
//! atoms support is supported here with nothing provider-specific added.

use std::fs;
use std::io::Read as _;

use nils_common::cli_contract::{Envelope, EnvelopeError, OutputFormat, exit, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendRunner};
use crate::cli::BINARY;
use crate::error::ForgeError;
use crate::ops::issue_view::{self, IssueViewPayload};
use crate::provider::ProviderContext;
use crate::tracker::Finding;

/// Label a program tracker issue carries.
pub const TRACKING_LABEL: &str = "workflow::tracking";

/// `error.code` of the failure envelope emitted when a tracker has findings.
pub const FINDINGS_ERROR_KIND: &str = "tracker_findings";

/// One finding as the commands report it: the grammar's `code` / `line` /
/// `ids` plus a message. Provider findings reuse the same shape.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrackerFinding {
    pub code: &'static str,
    /// 1-based line in the tracker body; `null` for a finding about the whole
    /// tracker.
    pub line: Option<usize>,
    pub ids: Vec<String>,
    pub message: String,
}

impl From<&Finding> for TrackerFinding {
    fn from(finding: &Finding) -> Self {
        Self {
            code: finding.code.as_str(),
            line: finding.line,
            ids: finding.ids.clone(),
            message: finding.message(),
        }
    }
}

/// What a command worked on: a provider issue, or a local draft body.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrackerTarget {
    /// `issue` or `body-file`.
    pub source: &'static str,
    pub provider: Option<&'static str>,
    pub number: Option<u64>,
    pub url: Option<String>,
}

impl TrackerTarget {
    pub fn issue(view: &IssueViewPayload) -> Self {
        Self {
            source: "issue",
            provider: Some(view.provider),
            number: Some(view.number),
            url: Some(view.url.clone()),
        }
    }

    pub fn draft() -> Self {
        Self {
            source: "body-file",
            provider: None,
            number: None,
            url: None,
        }
    }

    /// Short label for text output.
    pub fn describe(&self) -> String {
        match self.number {
            Some(number) => format!("issue #{number}"),
            None => "draft".to_string(),
        }
    }
}

/// A backend call a `--dry-run` would have made.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlannedAction {
    /// `edit-body` or `comment`.
    pub kind: &'static str,
    pub plan: Vec<String>,
}

impl PlannedAction {
    pub fn new(kind: &'static str, call: &BackendCall) -> Self {
        Self {
            kind,
            plan: call.plan_argv(),
        }
    }
}

/// Read the tracker issue through the `issue view` call.
pub fn read_issue<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    id: u64,
) -> Result<IssueViewPayload, ForgeError> {
    let output = runner.run(&issue_view::build_view_call(ctx, id))?;
    issue_view::parse_view_output(ctx, &output)
}

/// Read a `--body-file` draft; `-` reads stdin.
pub fn read_draft(path: &str) -> Result<String, ForgeError> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            ForgeError::software(
                schema_err(),
                "failed to read tracker body from stdin",
                Some(e.to_string()),
            )
        })?;
        return Ok(buf);
    }
    fs::read_to_string(path).map_err(|e| {
        ForgeError::software(
            schema_err(),
            format!("failed to read --body-file '{path}'"),
            Some(e.to_string()),
        )
    })
}

/// Emit a failure envelope that still carries the payload, so callers read the
/// findings from `data.findings[]` even though `ok = false`. Returns `DATA 65`.
pub fn emit_findings<T, F>(
    schema_version: String,
    payload: T,
    findings: &[TrackerFinding],
    format: OutputFormat,
    render_text: F,
) -> i32
where
    T: Serialize,
    F: FnOnce(&T),
{
    let message = format!("{} tracker finding(s)", findings.len());
    match format {
        OutputFormat::Json => {
            let envelope = Envelope {
                schema_version,
                ok: false,
                data: Some(payload),
                warnings: Vec::new(),
                error: Some(EnvelopeError::new(FINDINGS_ERROR_KIND, message)),
            };
            let serialized =
                serde_json::to_string(&envelope).unwrap_or_else(|_| String::from("{\"ok\":false}"));
            println!("{serialized}");
        }
        OutputFormat::Text => {
            eprintln!("error: {FINDINGS_ERROR_KIND}: {message}");
            render_text(&payload);
        }
    }
    exit::DATA
}

/// Print findings one per line for text output.
pub fn render_findings(findings: &[TrackerFinding]) {
    for finding in findings {
        let place = match finding.line {
            Some(line) => format!("line {line}"),
            None => "tracker".to_string(),
        };
        println!(
            "{place}: {code}: {message}",
            code = finding.code,
            message = finding.message
        );
    }
}

/// Neither an issue id nor `--body-file` was given. Clap rejects that first;
/// this keeps the ops total.
pub fn missing_target() -> ForgeError {
    ForgeError::software(
        schema_err(),
        "issue tracker: expected an issue id or --body-file",
        None,
    )
}

pub fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

/// In-memory forge for the `issue tracker` op tests: serves the gh-shaped
/// `issue view` / `issue edit` / comment calls the ops build, keeps a call
/// log, and can run a hook before every view to stand in for another session
/// editing the issue between two commands.
#[cfg(test)]
pub(crate) mod testing {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use serde_json::json;

    use crate::backend::{BackendCall, BackendRunner, BackendSuccess};
    use crate::cli::{GlobalFlags, ProviderFlag};
    use crate::error::ForgeError;
    use crate::provider::{DetectionSource, Provider, ProviderContext};

    pub(crate) const OWN_REPO: &str = "example/tracker";

    #[derive(Debug, Clone)]
    pub(crate) struct FakeIssue {
        pub body: String,
        pub state: &'static str,
        pub labels: Vec<&'static str>,
    }

    type ViewHook = Box<dyn Fn(&mut FakeIssue)>;

    #[derive(Default)]
    pub(crate) struct FakeForge {
        issues: RefCell<BTreeMap<(String, u64), FakeIssue>>,
        /// Planned argv of every call, in order (argv[0] is the program).
        pub calls: RefCell<Vec<Vec<String>>>,
        /// Body returned by each `issue view`, in order.
        pub served: RefCell<Vec<String>>,
        /// Comment bodies posted, in order.
        pub comments: RefCell<Vec<String>>,
        /// Make every comment call fail, as a provider outage would.
        pub fail_comments: Cell<bool>,
        before_view: RefCell<Option<ViewHook>>,
    }

    impl FakeForge {
        pub(crate) fn with_tracker(body: &str) -> Self {
            let forge = Self::default();
            forge.put(OWN_REPO, 1, body, "OPEN", &["workflow::tracking"]);
            forge
        }

        pub(crate) fn put(
            &self,
            repo: &str,
            number: u64,
            body: &str,
            state: &'static str,
            labels: &[&'static str],
        ) {
            self.issues.borrow_mut().insert(
                (repo.to_string(), number),
                FakeIssue {
                    body: body.to_string(),
                    state,
                    labels: labels.to_vec(),
                },
            );
        }

        /// Run `hook` on the issue at the start of every `issue view`.
        pub(crate) fn before_view(&self, hook: impl Fn(&mut FakeIssue) + 'static) {
            *self.before_view.borrow_mut() = Some(Box::new(hook));
        }

        pub(crate) fn body(&self, number: u64) -> String {
            self.issues.borrow()[&(OWN_REPO.to_string(), number)]
                .body
                .clone()
        }

        /// `"<verb> <number>"` for every call, e.g. `["view 1", "edit 1"]`.
        pub(crate) fn log(&self) -> Vec<String> {
            self.calls
                .borrow()
                .iter()
                .map(|argv| match argv[1].as_str() {
                    "api" => "comment".to_string(),
                    _ => format!("{} {}", argv[2], argv[3]),
                })
                .collect()
        }

        /// The `--body` of the n-th `issue edit` call.
        pub(crate) fn edited_body(&self, nth: usize) -> String {
            let calls = self.calls.borrow();
            let edit = calls
                .iter()
                .filter(|argv| argv[2] == "edit")
                .nth(nth)
                .expect("edit call");
            let flag = edit.iter().position(|arg| arg == "--body").expect("--body");
            edit[flag + 1].clone()
        }
    }

    fn flag<'a>(argv: &'a [String], name: &str) -> Option<&'a str> {
        argv.iter()
            .position(|arg| arg == name)
            .and_then(|at| argv.get(at + 1))
            .map(String::as_str)
    }

    impl BackendRunner for FakeForge {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let argv = call.plan_argv();
            self.calls.borrow_mut().push(argv.clone());
            let ok = |stdout: String| {
                Ok(BackendSuccess {
                    stdout,
                    stderr: String::new(),
                })
            };
            if argv[1] == "api" && self.fail_comments.get() {
                return Err(ForgeError::backend_error(
                    super::schema_err(),
                    "gh exited with status 1",
                    None,
                ));
            }
            if argv[1] == "api" {
                let body = flag(&argv, "--raw-field").expect("comment body");
                self.comments
                    .borrow_mut()
                    .push(body.trim_start_matches("body=").to_string());
                return ok("https://github.com/example/tracker/issues/1#issuecomment-9\n".into());
            }
            let number: u64 = argv[3].parse().expect("issue number");
            let repo = flag(&argv, "--repo").unwrap_or(OWN_REPO).to_string();
            let mut issues = self.issues.borrow_mut();
            let Some(issue) = issues.get_mut(&(repo.clone(), number)) else {
                return Err(ForgeError::backend_error(
                    super::schema_err(),
                    "gh exited with status 1",
                    Some("Could not resolve to an Issue".into()),
                ));
            };
            match argv[2].as_str() {
                // A state of `RATE_LIMITED` stands for a throttled provider.
                "view" if issue.state == "RATE_LIMITED" => Err(ForgeError::unavailable(
                    super::schema_err(),
                    crate::rate_limit::RATE_LIMITED_KIND,
                    "backend reports the GitHub API rate limit is exhausted",
                    None,
                )),
                "view" => {
                    if let Some(hook) = self.before_view.borrow().as_ref() {
                        hook(issue);
                    }
                    self.served.borrow_mut().push(issue.body.clone());
                    let labels: Vec<_> = issue.labels.iter().map(|l| json!({"name": l})).collect();
                    ok(json!({
                        "number": number,
                        "url": format!("https://github.com/{repo}/issues/{number}"),
                        "state": issue.state,
                        "title": "Tracker",
                        "body": issue.body,
                        "labels": labels,
                        "assignees": [],
                    })
                    .to_string())
                }
                "edit" => {
                    issue.body = flag(&argv, "--body").expect("--body").to_string();
                    ok(String::new())
                }
                other => panic!("FakeForge: unexpected call {other}: {argv:?}"),
            }
        }
    }

    pub(crate) fn ctx() -> ProviderContext {
        ProviderContext {
            provider: Provider::GitHub,
            host: "github.com".into(),
            source: DetectionSource::Flag,
            repo: Some(OWN_REPO.into()),
        }
    }

    pub(crate) fn global(dry_run: bool) -> GlobalFlags {
        GlobalFlags {
            format: None,
            remote: "origin".into(),
            provider: Some(ProviderFlag::Github),
            host: None,
            repo: Some(OWN_REPO.into()),
            store_root: None,
            dry_run,
        }
    }
}
