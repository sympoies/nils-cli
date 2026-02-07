use std::path::Path;
use std::time::Instant;

use nils_test_support::cmd::CmdOptions;

use crate::real_apps::matrix::{ScenarioOutcome, ScenarioStatus};
use crate::real_common::{self, UiPoint};

#[derive(Debug, Clone)]
pub struct ArcYoutubeProfile {
    pub app_name: String,
    pub youtube_home_url: String,
    pub video_tiles: Vec<UiPoint>,
}

impl ArcYoutubeProfile {
    pub fn from_default_profile() -> Self {
        let profile = real_common::load_profile();
        Self {
            app_name: "Arc".to_string(),
            youtube_home_url: profile.arc.youtube_home_url,
            video_tiles: profile.arc.video_tiles,
        }
    }
}

pub fn arc_youtube_opens_home_and_clicks_three_tiles(
    bin: &Path,
    options: &CmdOptions,
    artifact_dir: &Path,
    profile: &ArcYoutubeProfile,
) -> ScenarioOutcome {
    let started = Instant::now();
    let scenario_id = "arc_youtube_opens_home_and_clicks_three_tiles";
    let _ledger = real_common::begin_step_ledger(artifact_dir, scenario_id);
    let _cleanup_guard = real_common::ArcCleanupGuard::new();
    assert!(
        profile.video_tiles.len() >= 3,
        "arc profile requires at least 3 video tiles"
    );

    real_common::require_app_installed(&profile.app_name);
    let preflight = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "preflight"],
        "preflight",
    );
    real_common::require_preflight_ready(&preflight, &["accessibility", "automation"]);

    let mut screenshots = Vec::new();
    for (idx, point) in profile.video_tiles.iter().take(3).enumerate() {
        activate_arc(bin, options, &profile.app_name);
        open_youtube_home(bin, options, &profile.app_name, &profile.youtube_home_url);
        click(
            bin,
            options,
            point,
            &format!("arc click youtube tile {}", idx + 1),
        );
        wait_for_arc(bin, options, &profile.app_name);
        let checkpoint = artifact_dir.join(format!("arc-youtube-tile-{}.png", idx + 1));
        capture_active_window(bin, options, &checkpoint);
        screenshots.push(checkpoint.to_string_lossy().to_string());
    }

    let (step_ledger_path, failing_step_id, last_successful_step_id) =
        real_common::step_ledger_summary_for(artifact_dir);

    ScenarioOutcome {
        scenario_id: scenario_id.to_string(),
        status: ScenarioStatus::Passed,
        elapsed_ms: started.elapsed().as_millis() as u64,
        artifact_dir: artifact_dir.display().to_string(),
        screenshots,
        step_ledger_path,
        skip_reason: None,
        failing_step_id,
        last_successful_step_id,
    }
}

pub(crate) fn activate_arc(bin: &Path, options: &CmdOptions, app_name: &str) {
    real_common::ensure_input_source_for_text_entry();
    let payload = real_common::run_json_step(
        bin,
        options,
        &[
            "--format",
            "json",
            "window",
            "activate",
            "--app",
            app_name,
            "--wait-ms",
            "1800",
        ],
        "window.activate Arc",
    );
    assert_eq!(payload["command"], serde_json::json!("window.activate"));
    assert_eq!(
        payload["result"]["selected_app"]
            .as_str()
            .map(|value| value.eq_ignore_ascii_case(app_name)),
        Some(true)
    );
}

pub(crate) fn open_youtube_home(bin: &Path, options: &CmdOptions, app_name: &str, url: &str) {
    real_common::send_hotkey(
        bin,
        options,
        Some("cmd"),
        "l",
        "arc focus address bar for open youtube home",
    );
    real_common::replace_focused_text_with_clipboard(
        bin,
        options,
        url,
        "arc open youtube home url",
    );
    real_common::send_hotkey(bin, options, None, "return", "arc open youtube home");
    let wait_active = real_common::run_json_step(
        bin,
        options,
        &[
            "--format",
            "json",
            "wait",
            "app-active",
            "--app",
            app_name,
            "--timeout-ms",
            "7000",
            "--poll-ms",
            "60",
        ],
        "wait arc active after opening youtube home",
    );
    assert_eq!(wait_active["command"], serde_json::json!("wait.app-active"));

    verify_active_address_bar_url(bin, options);
}

pub(crate) fn click(bin: &Path, options: &CmdOptions, point: &UiPoint, step: &str) {
    let x = point.x.to_string();
    let y = point.y.to_string();
    let payload = real_common::run_json_step_with_retry(
        bin,
        options,
        &[
            "--format",
            "json",
            "input",
            "click",
            "--x",
            &x,
            "--y",
            &y,
            "--pre-wait-ms",
            "90",
            "--post-wait-ms",
            "150",
        ],
        step,
        2,
        std::time::Duration::from_millis(250),
    );
    assert_eq!(payload["command"], serde_json::json!("input.click"));
}

pub(crate) fn wait_for_arc(bin: &Path, options: &CmdOptions, app_name: &str) {
    let payload = real_common::run_json_step(
        bin,
        options,
        &[
            "--format",
            "json",
            "wait",
            "app-active",
            "--app",
            app_name,
            "--timeout-ms",
            "7000",
            "--poll-ms",
            "60",
        ],
        &format!("wait app-active {app_name}"),
    );
    assert_eq!(payload["command"], serde_json::json!("wait.app-active"));
}

pub(crate) fn capture_active_window(bin: &Path, options: &CmdOptions, screenshot_path: &Path) {
    let screenshot_path_text = screenshot_path.to_string_lossy().to_string();
    let payload = real_common::run_json_step(
        bin,
        options,
        &[
            "--format",
            "json",
            "observe",
            "screenshot",
            "--active-window",
            "--path",
            &screenshot_path_text,
        ],
        "observe screenshot",
    );
    assert_eq!(payload["command"], serde_json::json!("observe.screenshot"));
    assert!(screenshot_path.is_file(), "expected screenshot to exist");
}

fn verify_active_address_bar_url(bin: &Path, options: &CmdOptions) {
    real_common::send_hotkey(
        bin,
        options,
        Some("cmd"),
        "l",
        "arc focus address bar for URL verification",
    );
    real_common::send_hotkey(bin, options, Some("cmd"), "c", "arc copy address bar URL");
    let settle = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "150"],
        "wait after copying arc address bar URL",
    );
    assert_eq!(settle["command"], serde_json::json!("wait.sleep"));

    let current_url = real_common::read_clipboard_text();
    let normalized = current_url.to_ascii_lowercase();
    assert!(
        normalized.contains("youtube.com"),
        "expected Arc address bar to contain youtube.com, got `{current_url}`"
    );
    assert!(
        !normalized.contains("google.com/search"),
        "expected direct YouTube URL, got Google search URL `{current_url}`"
    );

    real_common::send_hotkey(
        bin,
        options,
        None,
        "escape",
        "arc dismiss address bar focus",
    );
}
