use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use jiff::Timestamp;
use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};

use super::{
    CliContext, SessionRecord, ShadowObservationView, TurnPhase, TurnState, now, session_dir,
};

pub(crate) const SHADOW_FILE: &str = "activity.shadow.json";
const SHADOW_LOCK_FILE: &str = ".activity.shadow.lock";
const SHADOW_VERSION: &str = "terminal-shadow.v1";
const SHADOW_DOCUMENT_VERSION: &str = "agent-session.activity-shadow.v1";
const SEMANTIC_STALE_AFTER_SECONDS: i64 = 5 * 60;
const SAMPLE_INTERVAL_SECONDS: i64 = 15;
const OBSERVER_TIMEOUT: Duration = Duration::from_millis(250);
const OBSERVER_OUTPUT_LIMIT: usize = 16 * 1024;
const CAPTURE_LINES: &str = "-20";
const MAX_CONCURRENT_SAMPLERS: usize = 4;
static ACTIVE_SAMPLERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ShadowDocument {
    schema_version: String,
    runtime_id: String,
    runtime_generation: u64,
    observation: ShadowObservationView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activity_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interrupt_since: Option<String>,
}

struct ShadowLock(fs::File);

impl Drop for ShadowLock {
    fn drop(&mut self) {
        // SAFETY: flock only observes the valid file descriptor owned by self.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

struct SamplerPermit;

impl Drop for SamplerPermit {
    fn drop(&mut self) {
        ACTIVE_SAMPLERS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn annotate_for_view(
    context: &CliContext,
    record: &SessionRecord,
    status: &str,
    tmux_bin: &Path,
    mut state: TurnState,
    schedule_sampling: bool,
) -> TurnState {
    if !eligible(&record.agent, status, &state) {
        state.shadow_observation = None;
        return state;
    }

    let dir = session_dir(context, &record.id);
    let path = dir.join(SHADOW_FILE);
    let sampled_state = state.clone();
    let cached = read_current(&path, record);
    if let Some(cached) = cached.as_ref().filter(|cached| {
        cached
            .activity_revision
            .is_none_or(|revision| revision == state.revision)
    }) {
        state.shadow_observation =
            Some(with_disagreement(cached.observation.clone(), &state.phase));
        project_interrupt_uncertainty(record, cached, &mut state);
    }
    if cached
        .as_ref()
        .is_some_and(|cached| is_recent(&cached.observation.observed_at, SAMPLE_INTERVAL_SECONDS))
    {
        return state;
    }

    if schedule_sampling {
        schedule_sample(
            context.clone(),
            record.clone(),
            tmux_bin.to_path_buf(),
            sampled_state,
        );
    }
    state
}

fn interrupt_rule(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("claude_interrupt_marker"),
        "codex" => Some("codex_interrupt_marker"),
        _ => None,
    }
}

fn project_interrupt_uncertainty(
    record: &SessionRecord,
    cached: &ShadowDocument,
    state: &mut TurnState,
) {
    if interrupt_rule(&record.agent) != Some(cached.observation.rule_id.as_str())
        || state.phase != TurnPhase::Working
        || cached.activity_revision != Some(state.revision)
        || !is_recent(&cached.observation.observed_at, SAMPLE_INTERVAL_SECONDS * 2)
        || state
            .current_turn
            .as_ref()
            .is_none_or(|turn| turn.attention.is_some())
    {
        return;
    }
    let Some(deadline) = cached
        .interrupt_since
        .as_deref()
        .and_then(|since| since.parse::<Timestamp>().ok())
        .and_then(|since| {
            since
                .checked_add(jiff::SignedDuration::from_secs(SAMPLE_INTERVAL_SECONDS))
                .ok()
        })
    else {
        return;
    };
    if !cached
        .observation
        .observed_at
        .parse::<Timestamp>()
        .is_ok_and(|observed| observed >= deadline)
    {
        return;
    }
    state.phase = TurnPhase::Unknown;
    state.phase_changed_at = deadline.to_string();
    state.source.kind = super::SourceKind::TerminalHeuristic;
    state.source.confidence = super::Confidence::Inferred;
    state.diagnostic = Some(super::ActivityDiagnosticView {
        reason: "interrupted_suspected".to_string(),
        extra: serde_json::Map::new(),
    });
}

fn schedule_sample(
    context: CliContext,
    record: SessionRecord,
    tmux_bin: std::path::PathBuf,
    state: TurnState,
) {
    let Some(permit) = sampler_permit() else {
        return;
    };
    thread::spawn(move || {
        let _permit = permit;
        let dir = session_dir(&context, &record.id);
        let path = dir.join(SHADOW_FILE);
        let Some(_lock) = acquire_shadow_lock(&dir) else {
            return;
        };
        let previous = read_current(&path, &record);
        if previous.as_ref().is_some_and(|cached| {
            is_recent(&cached.observation.observed_at, SAMPLE_INTERVAL_SECONDS)
        }) {
            return;
        }
        let observation = sample(&record, &tmux_bin, &state.phase);
        let Ok(current) = crate::load_session_record(&context, &record.id) else {
            return;
        };
        let Some(runtime) = current.runtime.as_ref() else {
            return;
        };
        let Some(sampled_runtime) = record.runtime.as_ref() else {
            return;
        };
        if runtime.launch_id != sampled_runtime.launch_id
            || runtime.generation != sampled_runtime.generation
            || current.tmux_session != record.tmux_session
            || super::state_for_view(&context, &current)
                .is_none_or(|current| current.revision != state.revision)
        {
            return;
        }
        let interrupt_since = (interrupt_rule(&record.agent) == Some(observation.rule_id.as_str()))
            .then(|| {
                previous
                    .as_ref()
                    .filter(|previous| {
                        previous.activity_revision == Some(state.revision)
                            && previous.observation.rule_id == observation.rule_id
                            && is_recent(
                                &previous.observation.observed_at,
                                SAMPLE_INTERVAL_SECONDS * 2,
                            )
                    })
                    .and_then(|previous| previous.interrupt_since.clone())
                    .unwrap_or_else(|| observation.observed_at.clone())
            });
        let document = ShadowDocument {
            schema_version: SHADOW_DOCUMENT_VERSION.to_string(),
            runtime_id: runtime.launch_id.clone(),
            runtime_generation: runtime.generation,
            observation,
            activity_revision: Some(state.revision),
            interrupt_since,
        };
        if let Ok(bytes) = serde_json::to_vec_pretty(&document) {
            let _ = write_atomic(&path, &bytes, SECRET_FILE_MODE);
        }
    });
}

fn sampler_permit() -> Option<SamplerPermit> {
    reserve_sampler(&ACTIVE_SAMPLERS, MAX_CONCURRENT_SAMPLERS).then_some(SamplerPermit)
}

fn reserve_sampler(counter: &AtomicUsize, limit: usize) -> bool {
    counter
        .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
            (active < limit).then_some(active + 1)
        })
        .is_ok()
}

fn acquire_shadow_lock(dir: &Path) -> Option<ShadowLock> {
    let path = dir.join(SHADOW_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(SECRET_FILE_MODE)
        .open(&path)
        .ok()?;
    fs::set_permissions(&path, fs::Permissions::from_mode(SECRET_FILE_MODE)).ok()?;
    // SAFETY: flock only observes the valid file descriptor owned by file.
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0)
        .then_some(ShadowLock(file))
}

fn eligible(provider: &str, status: &str, state: &TurnState) -> bool {
    if status != "running" || !matches!(provider, "claude" | "codex") {
        return false;
    }
    if state.phase == TurnPhase::Working
        && state
            .current_turn
            .as_ref()
            .is_some_and(|turn| turn.attention.is_none())
    {
        return true;
    }
    if state.phase == TurnPhase::Unknown {
        return true;
    }
    state.semantic_event.as_ref().map_or_else(
        || is_older_than(&state.phase_changed_at, SEMANTIC_STALE_AFTER_SECONDS),
        |event| is_older_than(&event.observed_at, SEMANTIC_STALE_AFTER_SECONDS),
    )
}

fn read_current(path: &Path, record: &SessionRecord) -> Option<ShadowDocument> {
    let bytes = fs::read(path).ok()?;
    let document = serde_json::from_slice::<ShadowDocument>(&bytes).ok()?;
    let runtime = record.runtime.as_ref()?;
    (document.schema_version == SHADOW_DOCUMENT_VERSION
        && document.runtime_id == runtime.launch_id
        && document.runtime_generation == runtime.generation
        && valid_observation(&document.observation))
    .then_some(document)
}

fn valid_observation(observation: &ShadowObservationView) -> bool {
    observation.observer_version == SHADOW_VERSION
        && matches!(
            observation.projection.as_str(),
            "working" | "needs_input" | "waiting" | "unknown"
        )
        && observation.observed_at.parse::<Timestamp>().is_ok()
        && observation.rule_id.len() <= 64
        && observation.rule_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn is_recent(value: &str, seconds: i64) -> bool {
    !is_older_than(value, seconds)
}

fn is_older_than(value: &str, seconds: i64) -> bool {
    let Ok(observed) = value.parse::<Timestamp>() else {
        return true;
    };
    Timestamp::now()
        .as_second()
        .saturating_sub(observed.as_second())
        >= seconds
}

fn sample(record: &SessionRecord, tmux_bin: &Path, phase: &TurnPhase) -> ShadowObservationView {
    let target = crate::managed_tmux_pane_target(&record.tmux_session);
    let title = command_output(
        tmux_bin,
        &["display-message", "-p", "-t", &target, "#{pane_title}"],
    );
    let bottom = command_output(
        tmux_bin,
        &["capture-pane", "-p", "-t", &target, "-S", CAPTURE_LINES],
    );
    let (projection, rule_id) = match (title, bottom) {
        (Some(title), Some(bottom)) => classify(&record.agent, &title, &bottom),
        _ => ("unknown", "observer_unavailable"),
    };
    ShadowObservationView {
        observer_version: SHADOW_VERSION.to_string(),
        rule_id: rule_id.to_string(),
        observed_at: now(),
        projection: projection.to_string(),
        disagrees: disagrees(phase, projection),
        extra: serde_json::Map::new(),
    }
}

fn command_output(tmux_bin: &Path, args: &[&str]) -> Option<String> {
    let mut command = ProcessCommand::new(tmux_bin);
    command.args(args);
    let output =
        crate::run_output_with_timeout_and_cap(command, OBSERVER_TIMEOUT, OBSERVER_OUTPUT_LIMIT)
            .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).to_string())
}

fn classify<'a>(provider: &str, title: &str, bottom: &str) -> (&'a str, &'a str) {
    let title = title.to_ascii_lowercase();
    let bottom = bottom.to_ascii_lowercase();
    match provider {
        "codex" if title.contains("action required") => {
            ("needs_input", "codex_action_required_title")
        }
        "codex" if bottom.contains("esc to interrupt") => ("working", "codex_working_indicator"),
        "codex" if bottom.contains("conversation interrupted") => {
            ("unknown", "codex_interrupt_marker")
        }
        "codex"
            if bottom.lines().rev().take(3).any(|line| {
                let line = line.trim_start();
                line.starts_with('›') || line.starts_with("> ")
            }) =>
        {
            ("waiting", "codex_prompt_visible")
        }
        "claude"
            if bottom.contains("do you want to proceed")
                || bottom.contains("allow this action") =>
        {
            ("needs_input", "claude_permission_form")
        }
        "claude" if bottom.contains("esc to interrupt") => ("working", "claude_working_indicator"),
        "claude" if claude_interrupt_composer_is_empty(&bottom) => {
            ("unknown", "claude_interrupt_marker")
        }
        "claude"
            if bottom.lines().rev().take(3).any(|line| {
                let line = line.trim_start();
                line.starts_with('❯') || line.starts_with("> ")
            }) =>
        {
            ("waiting", "claude_prompt_visible")
        }
        "codex" => ("unknown", "codex_unmatched"),
        "claude" => ("unknown", "claude_unmatched"),
        _ => ("unknown", "provider_unsupported"),
    }
}

fn claude_interrupt_composer_is_empty(bottom: &str) -> bool {
    let Some((_, after_marker)) =
        bottom.rsplit_once("interrupted · what should claude do instead?")
    else {
        return false;
    };
    // Renderer footers and padding may follow the composer. Only the latest
    // composer after the marker may establish idleness; an older one cannot.
    let mut next_nonempty = None;
    for line in after_marker.lines().rev().map(str::trim) {
        if line.starts_with('❯') || line == ">" || line.starts_with("> ") {
            // A multiline draft can start with an empty row. Any text before
            // the lower composer separator prevents an idle observation.
            return matches!(line, "❯" | ">")
                && next_nonempty.is_none_or(|next: &str| next.chars().all(|ch| ch == '─'));
        }
        if !line.is_empty() {
            next_nonempty = Some(line);
        }
    }
    false
}

pub(super) fn disagrees(phase: &TurnPhase, projection: &str) -> bool {
    match projection {
        "working" => *phase != TurnPhase::Working,
        "needs_input" => *phase != TurnPhase::NeedsInput,
        "waiting" => *phase != TurnPhase::Waiting,
        _ => false,
    }
}

fn with_disagreement(
    mut observation: ShadowObservationView,
    phase: &TurnPhase,
) -> ShadowObservationView {
    observation.disagrees = disagrees(phase, &observation.projection);
    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};
    use serde_json::{Map, json};
    use std::time::Instant;

    fn state(value: serde_json::Value) -> TurnState {
        serde_json::from_value(value).expect("state fixture")
    }

    fn record(id: &str, launch_id: &str, generation: u64) -> SessionRecord {
        serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1",
            "id": id,
            "agent": "codex",
            "mode": "interactive",
            "title": null,
            "cwd": "/tmp",
            "tmux_session": format!("hs-codex-{id}"),
            "prompt_file": null,
            "log_file": null,
            "created_at": "2026-07-10T00:00:00Z",
            "updated_at": "2026-07-10T00:00:00Z",
            "runtime": {
                "kind": "tmux",
                "tmux_session": format!("hs-codex-{id}"),
                "generation": generation,
                "started_at": "2026-07-10T00:00:00Z",
                "launch_id": launch_id
            }
        }))
        .expect("session record")
    }

    fn write_record(context: &CliContext, record: &SessionRecord) {
        let dir = session_dir(context, &record.id);
        fs::create_dir_all(&dir).expect("session dir");
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec_pretty(record).expect("record JSON"),
        )
        .expect("record write");
    }

    fn wait_for_shadow(path: &Path, record: &SessionRecord) -> ShadowDocument {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(document) = read_current(path, record) {
                return document;
            }
            assert!(Instant::now() < deadline, "shadow sample timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn claude_interrupt_marker_with_idle_composer_is_explicit_uncertainty() {
        assert_eq!(
            classify(
                "claude",
                "Claude",
                "Interrupted · What should Claude do instead?\n❯ "
            ),
            ("unknown", "claude_interrupt_marker")
        );
        assert_eq!(
            classify(
                "claude",
                "Claude",
                "Interrupted · What should Claude do instead?\nWorking… esc to interrupt\n❯ "
            ),
            ("working", "claude_working_indicator")
        );
        assert_eq!(
            classify(
                "claude",
                "Claude",
                "Interrupted · What should Claude do instead?\n❯ draft"
            ),
            ("waiting", "claude_prompt_visible")
        );
    }

    #[test]
    fn codex_interrupt_marker_is_uncertainty_even_with_composer_text() {
        // Codex capture text does not distinguish a renderer placeholder from
        // a draft. Neither can establish a completed or waiting turn.
        for composer in ["", "Describe a task", "draft"] {
            let pane = format!("Conversation interrupted\n› {composer}\n");
            assert_eq!(
                classify("codex", "Codex", &pane),
                ("unknown", "codex_interrupt_marker")
            );
        }
        let idle = "Conversation interrupted\n› \n";
        assert_eq!(
            classify(
                "codex",
                "Codex",
                &format!("Working (10s · esc to interrupt)\n{idle}")
            ),
            ("working", "codex_working_indicator")
        );
        assert_eq!(
            classify("codex", "Action Required", idle),
            ("needs_input", "codex_action_required_title")
        );
        assert_eq!(
            classify("codex", "Codex", "› Describe a task\n"),
            ("waiting", "codex_prompt_visible")
        );
    }

    #[test]
    fn codex_mutable_working_header_wins_over_stale_interrupt_marker() {
        let pane = "Thinking (10s • esc to interrupt)\nConversation interrupted\n› \n";
        assert_eq!(
            classify("codex", "Codex", pane),
            ("working", "codex_working_indicator")
        );
    }

    fn interrupted_pane_with_footer(composer: &str) -> String {
        format!(
            "Interrupted · What should Claude do instead?\n❯ {composer}\n\
             ────────────────────\n\
             Model: default · effort: normal\n\
             Working directory: <workspace>\n\
             Context: 10% · cache: enabled\n\
             Customizations: 1\n\
             Tools: 1 command\n\
             Tokens: 100\n\
             Permission mode: default · ? for shortcuts\n{}",
            "\n".repeat(8)
        )
    }

    #[test]
    fn claude_interrupt_marker_tolerates_renderer_footer_and_padding() {
        let pane = interrupted_pane_with_footer("");
        assert!(!pane.lines().rev().take(3).any(|line| line.trim() == "❯"));
        assert_eq!(
            classify("claude", "Claude", &pane),
            ("unknown", "claude_interrupt_marker")
        );
    }

    #[test]
    fn claude_interrupt_footer_preserves_latest_composer_and_work_guards() {
        let draft = interrupted_pane_with_footer("draft");
        let multiline_draft = interrupted_pane_with_footer("\n  draft continuation");
        let earlier_empty_composer = format!("❯ \n{draft}");
        let draft_after_empty =
            "Interrupted · What should Claude do instead?\n❯ \n❯ draft\nModel: default\n";
        let empty_before_marker =
            "❯ \nInterrupted · What should Claude do instead?\nModel: default\n";
        for pane in [
            &draft,
            &multiline_draft,
            &earlier_empty_composer,
            draft_after_empty,
            empty_before_marker,
        ] {
            assert_ne!(
                classify("claude", "Claude", pane).1,
                "claude_interrupt_marker"
            );
        }
        let idle = interrupted_pane_with_footer("");
        assert_eq!(
            classify(
                "claude",
                "Claude",
                &format!("Working… esc to interrupt\n{idle}")
            ),
            ("working", "claude_working_indicator")
        );
        assert_eq!(
            classify("claude", "Claude", &format!("Allow this action?\n{idle}")),
            ("needs_input", "claude_permission_form")
        );
    }

    #[test]
    fn claude_sustained_interrupt_projection_is_turn_and_runtime_fenced() {
        sustained_interrupt_projection_is_turn_and_runtime_fenced("claude");
    }

    #[test]
    fn codex_sustained_interrupt_projection_is_turn_and_runtime_fenced() {
        sustained_interrupt_projection_is_turn_and_runtime_fenced("codex");
    }

    fn sustained_interrupt_projection_is_turn_and_runtime_fenced(provider: &str) {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut record = record("interrupt", "launch-a", 1);
        record.agent = provider.to_string();
        write_record(&context, &record);
        let working = state(json!({
            "schema_version": "agent-session.turn-state.v1", "phase": "working",
            "phase_changed_at": now(), "revision": 10,
            "source": {"kind": "provider_hook", "provider": provider, "confidence": "observed"},
            "semantic_event": {"kind": "progress", "observed_at": now()},
            "current_turn": {"provider_turn_id": "turn-1", "started_at": now()}
        }));
        let earlier = Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_secs(20))
            .unwrap()
            .to_string();
        let path = session_dir(&context, &record.id).join(SHADOW_FILE);
        let mut cached = json!({
            "schema_version": SHADOW_DOCUMENT_VERSION, "runtime_id": "launch-a", "runtime_generation": 1,
            "activity_revision": 10, "interrupt_since": earlier,
            "observation": {"observer_version": SHADOW_VERSION, "rule_id": format!("{provider}_interrupt_marker"),
                "observed_at": now(), "projection": "unknown", "disagrees": false}
        });
        let view = |state: TurnState, fixture: &serde_json::Value| {
            fs::write(&path, serde_json::to_vec(fixture).unwrap()).unwrap();
            annotate_for_view(
                &context,
                &record,
                "running",
                Path::new("unused-tmux"),
                state,
                false,
            )
        };
        let uncertain = view(working.clone(), &cached);
        assert_eq!(uncertain.phase, TurnPhase::Unknown);
        assert_eq!(
            uncertain.diagnostic.as_ref().unwrap().reason,
            "interrupted_suspected"
        );
        assert_eq!(uncertain.current_turn, working.current_turn);
        assert_eq!(uncertain.last_turn, working.last_turn);
        assert_eq!(uncertain.revision, working.revision);
        assert_eq!(
            uncertain.source.kind,
            super::super::SourceKind::TerminalHeuristic
        );
        assert_eq!(
            uncertain.source.confidence,
            super::super::Confidence::Inferred
        );
        let mut next_prompt = working.clone();
        next_prompt.revision += 1;
        next_prompt.current_turn.as_mut().unwrap().provider_turn_id = Some("turn-2".to_string());
        assert_eq!(view(next_prompt, &cached).phase, TurnPhase::Working);
        cached["runtime_id"] = json!("old-launch");
        assert_eq!(view(working.clone(), &cached).phase, TurnPhase::Working);
        cached["runtime_id"] = json!("launch-a");
        cached["runtime_generation"] = json!(2);
        assert_eq!(view(working.clone(), &cached).phase, TurnPhase::Working);
        cached["runtime_generation"] = json!(1);
        cached["observation"]["rule_id"] = json!(if provider == "codex" {
            "claude_interrupt_marker"
        } else {
            "codex_interrupt_marker"
        });
        assert_eq!(view(working.clone(), &cached).phase, TurnPhase::Working);
        cached["observation"]["rule_id"] = json!(format!("{provider}_interrupt_marker"));
        cached["interrupt_since"] = json!(
            Timestamp::now()
                .checked_sub(jiff::SignedDuration::from_secs(70))
                .unwrap()
                .to_string()
        );
        cached["observation"]["observed_at"] = json!(
            Timestamp::now()
                .checked_sub(jiff::SignedDuration::from_secs(40))
                .unwrap()
                .to_string()
        );
        assert_eq!(view(working.clone(), &cached).phase, TurnPhase::Working);
        cached["observation"]["observed_at"] = json!(now());
        cached["interrupt_since"] = json!(now());
        assert_eq!(view(working.clone(), &cached).phase, TurnPhase::Working);
        cached["interrupt_since"] = json!(earlier);
        let mut attention = working.clone();
        attention.current_turn.as_mut().unwrap().attention = Some(
            serde_json::from_value(json!({
                "kind": "approval", "requested_at": now(), "pending_count": 1,
                "certainty": "conservative"
            }))
            .unwrap(),
        );
        assert_eq!(view(attention, &cached).phase, TurnPhase::Working);
        cached["observation"]["rule_id"] = json!(format!("{provider}_working_indicator"));
        cached["observation"]["projection"] = json!("working");
        assert_eq!(view(working, &cached).phase, TurnPhase::Working);
    }

    #[test]
    fn claude_shadow_writer_sustains_resets_and_throttles_across_revisions() {
        shadow_writer_sustains_resets_and_throttles_across_revisions("claude");
    }

    #[test]
    fn codex_shadow_writer_sustains_resets_and_throttles_across_revisions() {
        shadow_writer_sustains_resets_and_throttles_across_revisions("codex");
    }

    fn shadow_writer_sustains_resets_and_throttles_across_revisions(provider: &str) {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut record = record("writer", "launch-a", 1);
        record.agent = provider.to_string();
        write_record(&context, &record);
        crate::activity::activate_runtime(&context, &record).unwrap();
        let progress = |name: &str| {
            let event: super::super::TurnEvent = serde_json::from_value(json!({
                "schema_version": "agent-session.turn-event.v1", "event_id": name,
                "runtime_id": "launch-a", "provider": provider, "provider_turn_id": name,
                "kind": "progress", "confidence": "observed", "source_kind": "provider_hook"
            }))
            .unwrap();
            crate::activity::ingest_event(&context, &record.id, event).unwrap();
        };
        progress("start");
        let counter = tmp.path().join("captures");
        let tmux = tmp.path().join("fake-tmux");
        fs::write(&tmux, format!(
            "#!/bin/sh\ncase \"$1\" in\n display-message) printf 'Claude\\n' ;;\n capture-pane) printf 'capture\\n' >> {}; printf '%s' {} ;;\nesac\n",
            shell_words::quote(counter.to_str().unwrap()),
            shell_words::quote(&if provider == "claude" { interrupted_pane_with_footer("") } else { "Conversation interrupted\n› \n".to_string() }),
        )).unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let path = session_dir(&context, &record.id).join(SHADOW_FILE);
        let collect = || {
            annotate_for_view(
                &context,
                &record,
                "running",
                &tmux,
                crate::activity::state_for_view(&context, &record).unwrap(),
                true,
            )
        };
        collect();
        let first = wait_for_shadow(&path, &record);
        assert_eq!(
            first.interrupt_since.as_ref(),
            Some(&first.observation.observed_at)
        );
        progress("next-progress");
        collect();
        thread::sleep(Duration::from_millis(80));
        assert_eq!(
            fs::read_to_string(&counter).unwrap().lines().count(),
            1,
            "a newer revision bypassed the sampling throttle"
        );

        let age = |mut document: ShadowDocument, seconds: i64| {
            let deadline = Instant::now() + Duration::from_secs(2);
            let _lock = loop {
                if let Some(lock) = acquire_shadow_lock(&session_dir(&context, &record.id)) {
                    break lock;
                }
                assert!(
                    Instant::now() < deadline,
                    "sampler did not release its lock"
                );
                thread::sleep(Duration::from_millis(10));
            };
            let earlier = Timestamp::now()
                .checked_sub(jiff::SignedDuration::from_secs(seconds))
                .unwrap()
                .to_string();
            document.observation.observed_at = earlier.clone();
            document.interrupt_since = Some(earlier.clone());
            fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
            earlier
        };
        age(first, 20);
        collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        let second = loop {
            let document = read_current(&path, &record).unwrap();
            if is_recent(&document.observation.observed_at, 2) {
                break document;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            second.interrupt_since.as_ref(),
            Some(&second.observation.observed_at),
            "a new hook must reset the sustained window"
        );
        let since = age(second, 20);
        collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        let third = loop {
            let document = read_current(&path, &record).unwrap();
            if is_recent(&document.observation.observed_at, 2) {
                break document;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(third.interrupt_since.as_deref(), Some(since.as_str()));
        let mut state = crate::activity::state_for_view(&context, &record).unwrap();
        project_interrupt_uncertainty(&record, &third, &mut state);
        assert_eq!(state.phase, TurnPhase::Unknown);
        assert_eq!(collect().phase, TurnPhase::Unknown);
        let persisted = crate::activity::state_for_view(&context, &record).unwrap();
        assert_eq!(persisted.phase, TurnPhase::Working);
        assert_eq!(persisted.current_turn, state.current_turn);
        assert_eq!(persisted.last_turn, state.last_turn);
        age(third, 40);
        assert_eq!(
            annotate_for_view(
                &context,
                &record,
                "running",
                &tmux,
                crate::activity::state_for_view(&context, &record).unwrap(),
                false
            )
            .phase,
            TurnPhase::Working
        );
        collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        let reset = loop {
            let document = read_current(&path, &record).unwrap();
            if is_recent(&document.observation.observed_at, 2) {
                break document;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            reset.interrupt_since.as_ref(),
            Some(&reset.observation.observed_at)
        );
    }

    #[test]
    fn classifier_uses_bounded_rule_ids_without_returning_content() {
        assert_eq!(
            classify("codex", "Action Required", "private prompt"),
            ("needs_input", "codex_action_required_title")
        );
        assert_eq!(
            classify("claude", "Claude", "Working… esc to interrupt"),
            ("working", "claude_working_indicator")
        );
        assert_eq!(
            classify("codex", "Codex", "unrecognized private output"),
            ("unknown", "codex_unmatched")
        );
    }

    #[test]
    fn exact_semantic_phase_wins_every_shadow_disagreement() {
        assert!(disagrees(&TurnPhase::Working, "waiting"));
        assert!(disagrees(&TurnPhase::NeedsInput, "working"));
        assert!(!disagrees(&TurnPhase::Waiting, "waiting"));
        assert!(!disagrees(&TurnPhase::Working, "unknown"));
    }

    #[test]
    fn sampling_policy_targets_only_uncertain_or_semantically_stale_sessions() {
        let fresh = state(json!({
            "schema_version": "agent-session.turn-state.v1",
            "phase": "working",
            "phase_changed_at": now(),
            "revision": 2,
            "source": {
                "kind": "provider_hook",
                "provider": "codex",
                "confidence": "authoritative"
            },
            "semantic_event": {
                "kind": "progress",
                "observed_at": now()
            }
        }));
        let mut unknown = fresh.clone();
        unknown.phase = TurnPhase::Unknown;
        unknown.semantic_event = None;
        unknown.extra = Map::new();

        assert!(!eligible("codex", "running", &fresh));
        assert!(eligible("codex", "running", &unknown));
        assert!(eligible("claude", "running", &unknown));
        assert!(!eligible("dsh", "running", &unknown));
        assert!(!eligible("codex", "stopped", &unknown));
    }

    #[test]
    fn shadow_sampler_has_a_process_wide_concurrency_cap() {
        let counter = AtomicUsize::new(0);
        for _ in 0..MAX_CONCURRENT_SAMPLERS {
            assert!(reserve_sampler(&counter, MAX_CONCURRENT_SAMPLERS));
        }
        assert!(!reserve_sampler(&counter, MAX_CONCURRENT_SAMPLERS));
    }

    #[test]
    fn shadow_cache_is_bound_to_launch_identity_not_generation_alone() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join(SHADOW_FILE);
        let original = record("same-id", "launch-a", 1);
        let replacement = record("same-id", "launch-b", 1);
        let document = ShadowDocument {
            schema_version: SHADOW_DOCUMENT_VERSION.to_string(),
            runtime_id: "launch-a".to_string(),
            runtime_generation: 1,
            activity_revision: None,
            interrupt_since: None,
            observation: ShadowObservationView {
                observer_version: SHADOW_VERSION.to_string(),
                rule_id: "codex_prompt_visible".to_string(),
                observed_at: now(),
                projection: "waiting".to_string(),
                disagrees: false,
                extra: Map::new(),
            },
        };
        fs::write(&path, serde_json::to_vec(&document).expect("shadow JSON"))
            .expect("shadow write");

        assert!(read_current(&path, &original).is_some());
        assert!(read_current(&path, &replacement).is_none());
    }

    #[test]
    fn list_and_serve_collectors_own_distinct_shadow_sampling_lifecycles() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let record = record("shadow-integration", "launch-a", 1);
        write_record(&context, &record);
        crate::activity::activate_runtime(&context, &record).expect("activity state");
        let tmux = tmp.path().join("fake-tmux");
        fs::write(
            &tmux,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  list-windows) printf '%s\\t100\\n' {} ;;\n  display-message) printf 'Codex\\n' ;;\n  capture-pane) printf '› \\n' ;;\nesac\n",
                shell_words::quote(&record.tmux_session),
            ),
        )
        .expect("fake tmux");
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).expect("tmux mode");
        let path = session_dir(&context, &record.id).join(SHADOW_FILE);

        let one_shot = crate::list_sessions(&context, Some(&tmux)).expect("one-shot list");
        assert_eq!(one_shot.len(), 1);
        thread::sleep(Duration::from_millis(50));
        assert!(!path.exists(), "one-shot view launched detached sampling");

        let immediate =
            crate::list_sessions_for_serve(&context, Some(&tmux)).expect("serve collection");
        assert_eq!(immediate.len(), 1);
        assert!(
            immediate[0]
                .turn_state
                .as_ref()
                .is_some_and(|state| state.shadow_observation.is_none())
        );
        let sampled = wait_for_shadow(&path, &record);
        assert_eq!(sampled.observation.projection, "waiting");
        assert_eq!(sampled.observation.rule_id, "codex_prompt_visible");
        assert_eq!(sampled.runtime_id, "launch-a");
        let cached =
            crate::list_sessions_for_serve(&context, Some(&tmux)).expect("cached collection");
        assert!(
            cached[0]
                .turn_state
                .as_ref()
                .and_then(|state| state.shadow_observation.as_ref())
                .is_some_and(|observation| observation.projection == "waiting")
        );
    }
}
