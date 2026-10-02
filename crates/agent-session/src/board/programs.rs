//! `GET /board/programs/v1` (`session-board-v1`, "Programs").
//!
//! The daemon names the work-mode program of every session on this machine
//! and serves each program's lanes, read through `forge-cli issue tracker
//! show`, which owns the tracker grammar. Reads are cached, refreshed lazily
//! when the route is read, and the last good copy is kept, marked stale, when
//! a refresh fails.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::lineage::WorkRef;
use crate::usage::run_helper;
use crate::{CliContext, CliError};

pub(crate) const PROGRAMS_SCHEMA: &str = "agent-session.board-programs.v1";
const TRACKER_SCHEMA: &str = "cli.forge-cli.issue.tracker.show.v1";
/// `forge-cli` binary override, for hosts where it is not on `PATH`.
pub(crate) const FORGE_CLI_ENV: &str = "AGENT_SESSION_FORGE_CLI_BIN";
/// A copy younger than this is served without a refresh.
const TTL: Duration = Duration::from_secs(5 * 60);
const HELPER_TIMEOUT: Duration = Duration::from_secs(10);
/// One refresh pass reads for at most this long in total; programs it does
/// not reach keep their cached copy, marked stale, and are tried first by the
/// next read.
const REFRESH_BUDGET: Duration = Duration::from_secs(20);
const OUTPUT_LIMIT: u64 = 1024 * 1024;
/// At most this many programs per snapshot, in reference order.
const MAX_PROGRAMS: usize = 16;
/// A read needs at least this much of the budget left to start.
const MIN_READ: Duration = Duration::from_millis(500);
const MAX_ROWS: usize = 500;

/// One program's last read. A failed read is remembered too, so a failing
/// `forge-cli` is retried once per TTL and not once per request.
struct Cached {
    attempted: Instant,
    /// The last good read, minus `stale`; `None` when none ever succeeded.
    body: Option<Value>,
    /// Whether the last attempt failed and `body` is an older copy.
    stale: bool,
}

/// The program cache of one daemon.
pub(crate) struct ProgramCache {
    forge_cli: String,
    ttl: Duration,
    budget: Duration,
    entries: Mutex<BTreeMap<String, Cached>>,
    /// Held while refreshing, so concurrent reads share one refresh.
    refresh: Mutex<()>,
}

impl ProgramCache {
    pub(crate) fn from_environment() -> Self {
        Self::new(
            crate::non_empty_env(FORGE_CLI_ENV).unwrap_or_else(|| "forge-cli".to_string()),
            TTL,
        )
    }

    fn new(forge_cli: String, ttl: Duration) -> Self {
        Self {
            forge_cli,
            ttl,
            budget: REFRESH_BUDGET,
            entries: Mutex::new(BTreeMap::new()),
            refresh: Mutex::new(()),
        }
    }

    /// `agent-session.board-programs.v1` for the programs this machine's
    /// sessions name. A program that has never been read successfully is
    /// omitted.
    pub(crate) fn snapshot(&self, context: &CliContext, machine: &str) -> Result<Value, CliError> {
        let wanted = program_refs(context);
        let is_fresh = |entries: &BTreeMap<String, Cached>, reference: &str| {
            entries
                .get(reference)
                .is_some_and(|cached| cached.attempted.elapsed() < self.ttl)
        };
        let any_stale = {
            let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            wanted
                .iter()
                .any(|(reference, _)| !is_fresh(&entries, reference))
        };
        if any_stale {
            let _refreshing = self.refresh.lock().unwrap_or_else(|e| e.into_inner());
            let deadline = Instant::now() + self.budget;
            for (reference, work_ref) in &wanted {
                let fresh = {
                    let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
                    is_fresh(&entries, reference)
                };
                if fresh {
                    continue;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining < MIN_READ {
                    let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(cached) = entries.get_mut(reference) {
                        cached.stale = true;
                    }
                    continue;
                }
                let read = self.read_tracker(reference, work_ref, remaining.min(HELPER_TIMEOUT));
                let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
                let previous = entries.remove(reference).and_then(|cached| cached.body);
                let cached = match read {
                    Some(body) => Cached {
                        attempted: Instant::now(),
                        body: Some(body),
                        stale: false,
                    },
                    None => Cached {
                        attempted: Instant::now(),
                        body: previous,
                        stale: true,
                    },
                };
                entries.insert(reference.clone(), cached);
            }
        }
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|reference, _| wanted.contains_key(reference));
        let programs: Vec<Value> = wanted
            .keys()
            .filter_map(|reference| entries.get(reference))
            .filter_map(|cached| {
                let mut program = cached.body.clone()?;
                program["stale"] = json!(cached.stale);
                Some(program)
            })
            .collect();
        Ok(json!({
            "schema_version": PROGRAMS_SCHEMA,
            "machine": machine,
            "generated_at": jiff::Timestamp::now().to_string(),
            "programs": programs,
        }))
    }

    /// One tracker read as the served program object, or `None` on any
    /// failure: the helper cannot run, times out, or answers anything other
    /// than a tracker.
    fn read_tracker(
        &self,
        reference: &str,
        work_ref: &WorkRef,
        timeout: Duration,
    ) -> Option<Value> {
        let target = format!("{}#{}", work_ref.repository, work_ref.number);
        let output = run_helper(
            &self.forge_cli,
            &[
                "--provider",
                &work_ref.provider,
                "issue",
                "tracker",
                "show",
                &target,
                "--format",
                "json",
            ],
            &[],
            timeout,
            OUTPUT_LIMIT,
        )
        .ok()?;
        if output.status != Some(0) {
            return None;
        }
        let envelope: Value = serde_json::from_slice(&output.stdout).ok()?;
        if envelope["schema_version"] != TRACKER_SCHEMA || envelope["ok"] != true {
            return None;
        }
        project_tracker(reference, &envelope["data"])
    }
}

/// The `{ref, url, title, state, fetched_at, rows}` of a tracker read.
fn project_tracker(reference: &str, data: &Value) -> Option<Value> {
    let text = |value: &Value| value.as_str().map(str::to_string);
    let rows = data["rows"].as_array()?;
    if rows.len() > MAX_ROWS {
        return None;
    }
    let rows: Vec<Value> = rows
        .iter()
        .filter_map(|row| {
            Some(json!({
                "id": text(&row["id"])?,
                "title": text(&row["title"])?,
                "reference": text(&row["reference"]),
                "done": row["done"].as_bool()?,
                "phase": text(&row["phase"]),
                "after": row["after"]
                    .as_array()
                    .map(|after| after.iter().filter_map(text).collect::<Vec<_>>())
                    .unwrap_or_default(),
                "notes": text(&row["notes"]),
            }))
        })
        .collect();
    Some(json!({
        "ref": reference,
        "url": text(&data["url"]),
        "title": text(&data["title"])?,
        "state": text(&data["state"]),
        "fetched_at": jiff::Timestamp::now().to_string(),
        "rows": rows,
    }))
}

/// The distinct programs named by the sessions in this state directory, keyed
/// by reference (`owner/repo#N`, with a `gitlab:` prefix for GitLab).
fn program_refs(context: &CliContext) -> BTreeMap<String, WorkRef> {
    let mut programs = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(context.state_dir.join("sessions")) else {
        return programs;
    };
    let ids: BTreeSet<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .collect();
    for id in ids {
        let Some(program) = crate::load_session_record(context, &id)
            .ok()
            .and_then(|record| record.work)
            .and_then(|work| work.program)
        else {
            continue;
        };
        programs.insert(program.display(), program);
    }
    // The first MAX_PROGRAMS by reference, whatever the session ids are.
    while programs.len() > MAX_PROGRAMS {
        programs.pop_last();
    }
    programs
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn session_with_program(context: &CliContext, id: &str, program: Option<Value>) {
        let mut value = json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION,
            "id": id,
            "agent": "codex",
            "mode": "interactive",
            "cwd": "/srv/x",
            "tmux_session": format!("agent-{id}"),
            "prompt_file": null,
            "log_file": null,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:04:00Z",
        });
        if let Some(program) = program {
            value["work"] =
                json!({"program": program, "issues": [], "inherited": false, "revision": 1});
        }
        let record: crate::SessionRecord = serde_json::from_value(value).expect("record");
        std::fs::create_dir_all(crate::session_dir(context, id)).expect("session dir");
        crate::write_session_record(context, &record).expect("write record");
    }

    fn github(repository: &str, number: u64) -> Value {
        json!({"provider": "github", "repository": repository, "number": number})
    }

    /// A `forge-cli` stand-in: it counts its calls in `calls`, answers a
    /// tracker titled after its target, and fails after counting while `fail`
    /// exists.
    fn fake_forge(dir: &std::path::Path) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("forge-cli");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho \"$@\" >> {dir}/calls\n[ -e {dir}/fail ] && exit 1\n\
                 for last; do :; done; target=$(echo \"$@\" | sed 's/.*show \\([^ ]*\\) .*/\\1/')\n\
                 printf '{{\"schema_version\":\"{TRACKER_SCHEMA}\",\"ok\":true,\"data\":{{\"title\":\"%s\",\"state\":\"open\",\"url\":\"https://example.test/%s\",\"rows\":[{{\"id\":\"A1\",\"title\":\"Lane\",\"reference\":null,\"done\":false,\"phase\":null,\"after\":[],\"notes\":null}}]}}}}' \"$target\" \"$target\"\n",
                dir = dir.display()
            ),
        )
        .expect("fake forge-cli");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("mode");
        path.to_string_lossy().to_string()
    }

    fn calls(dir: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(dir.join("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn programs_are_read_once_per_ttl_and_kept_stale_when_a_refresh_fails() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        session_with_program(&context, "a", Some(github("o/program", 44)));
        session_with_program(&context, "b", Some(github("o/program", 44)));
        session_with_program(&context, "c", Some(github("o/other", 7)));
        session_with_program(&context, "no-work", None);
        let forge = fake_forge(tmp.path());
        let forge_path = forge.clone();

        let cache = ProgramCache::new(forge.clone(), Duration::from_secs(3600));
        let first = cache.snapshot(&context, "host-a").expect("snapshot");
        assert_eq!(first["schema_version"], PROGRAMS_SCHEMA);
        assert_eq!(first["machine"], "host-a");
        let refs: Vec<&str> = first["programs"]
            .as_array()
            .expect("programs")
            .iter()
            .map(|program| program["ref"].as_str().expect("ref"))
            .collect();
        assert_eq!(refs, vec!["o/other#7", "o/program#44"]);
        assert_eq!(first["programs"][1]["title"], "o/program#44");
        assert_eq!(first["programs"][1]["stale"], false);
        assert_eq!(first["programs"][1]["rows"][0]["id"], "A1");
        assert_eq!(
            calls(tmp.path()),
            vec![
                "--provider github issue tracker show o/other#7 --format json",
                "--provider github issue tracker show o/program#44 --format json"
            ]
        );

        // Within the TTL nothing is read again.
        cache.snapshot(&context, "host-a").expect("again");
        assert_eq!(calls(tmp.path()).len(), 2);

        // Past the TTL a failing read keeps the last good copy, marked stale.
        let cache = ProgramCache::new(forge, Duration::ZERO);
        cache.snapshot(&context, "host-a").expect("warm");
        std::fs::write(tmp.path().join("fail"), b"").expect("fail flag");
        let stale = cache.snapshot(&context, "host-a").expect("stale");
        let programs = stale["programs"].as_array().expect("programs");
        assert_eq!(programs.len(), 2);
        assert!(programs.iter().all(|program| program["stale"] == true));
        assert_eq!(programs[1]["rows"][0]["id"], "A1");

        // A pass whose budget is spent skips the programs it does not reach
        // and serves their cached copies as stale.
        std::fs::remove_file(tmp.path().join("fail")).expect("clear fail flag");
        let mut spent = ProgramCache::new(forge_path.clone(), Duration::ZERO);
        spent.snapshot(&context, "host-a").expect("warm");
        spent.budget = Duration::ZERO;
        let before = calls(tmp.path()).len();
        let served = spent.snapshot(&context, "host-a").expect("spent");
        assert_eq!(
            calls(tmp.path()).len(),
            before,
            "no read starts without budget"
        );
        let programs = served["programs"].as_array().expect("programs");
        assert_eq!(programs.len(), 2);
        assert!(programs.iter().all(|program| program["stale"] == true));
        std::fs::write(tmp.path().join("fail"), b"").expect("fail flag");

        // A forge that keeps failing is tried once per TTL, not per request.
        let failing = ProgramCache::new(forge_path, Duration::from_secs(3600));
        let before = calls(tmp.path()).len();
        for _ in 0..3 {
            let served = failing.snapshot(&context, "host-a").expect("failing");
            assert_eq!(served["programs"], json!([]));
        }
        assert_eq!(
            calls(tmp.path()).len(),
            before + 2,
            "one attempt per program"
        );

        // A program no session names any more is dropped; one that never
        // read successfully is omitted.
        std::fs::remove_dir_all(crate::session_dir(&context, "c")).expect("remove c");
        session_with_program(&context, "d", Some(github("o/never", 1)));
        let cold = ProgramCache::new(
            tmp.path().join("missing").to_string_lossy().to_string(),
            TTL,
        );
        let omitted = cold.snapshot(&context, "host-a").expect("omitted");
        assert_eq!(omitted["programs"], json!([]));
        let after = cache.snapshot(&context, "host-a").expect("after");
        let refs: Vec<&str> = after["programs"]
            .as_array()
            .expect("programs")
            .iter()
            .map(|program| program["ref"].as_str().expect("ref"))
            .collect();
        assert_eq!(refs, vec!["o/program#44"]);
    }

    #[test]
    fn a_tracker_is_projected_without_its_labels_findings_or_line_numbers() {
        let data = json!({
            "source": "issue", "provider": "github", "number": 44,
            "url": "https://example.test/o/program/issues/44",
            "repo": "o/program", "title": "Program", "state": "open",
            "labels": ["type::feature"], "row_count": 3,
            "rows": [
                {"id": "A1", "title": "Lane", "reference": "o/lane#1", "done": false,
                 "phase": "Phase 1", "after": [], "notes": null, "line": 12},
                {"id": "A2", "title": "Gate", "reference": null, "done": true,
                 "phase": null, "after": ["A1"], "notes": "n", "line": 13},
                {"id": 3, "title": "malformed row is dropped", "done": false}
            ],
            "findings": [{"code": "x"}]
        });
        let program = project_tracker("o/program#44", &data).expect("program");
        assert_eq!(program["ref"], "o/program#44");
        assert_eq!(program["url"], "https://example.test/o/program/issues/44");
        assert_eq!(program["title"], "Program");
        assert_eq!(program["state"], "open");
        assert!(program["fetched_at"].as_str().is_some());
        assert_eq!(
            program["rows"],
            json!([
                {"id": "A1", "title": "Lane", "reference": "o/lane#1", "done": false,
                 "phase": "Phase 1", "after": [], "notes": null},
                {"id": "A2", "title": "Gate", "reference": null, "done": true,
                 "phase": null, "after": ["A1"], "notes": "n"}
            ])
        );
        assert_eq!(project_tracker("o/p#1", &json!({"rows": []})), None);
    }
}
