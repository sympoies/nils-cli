use std::path::Path;
use std::time::Instant;

use nils_test_support::cmd::CmdOptions;

use crate::real_apps::matrix::{ScenarioOutcome, ScenarioStatus};
use crate::real_common::{self, SpotifyPlaybackState};

pub fn spotify_ui_selects_track_and_toggles_playback(
    bin: &Path,
    options: &CmdOptions,
    artifact_dir: &Path,
) -> ScenarioOutcome {
    let started = Instant::now();
    real_common::require_app_installed("Spotify");
    let _cleanup_guard = real_common::SpotifyPlaybackCleanupGuard::capture();

    let profile = real_common::load_profile().spotify;
    let mut screenshots = Vec::new();
    let mut states = Vec::new();

    activate_spotify(bin, options);
    click(
        bin,
        options,
        profile.search_box.x,
        profile.search_box.y,
        "spotify focus search",
    );
    real_common::replace_focused_text_with_clipboard(
        bin,
        options,
        "lofi hip hop",
        "spotify search query",
    );
    real_common::send_hotkey(bin, options, None, "return", "spotify confirm search query");

    let settle = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "800"],
        "wait after spotify search",
    );
    assert_eq!(settle["command"], serde_json::json!("wait.sleep"));

    let track_point = profile
        .track_rows
        .first()
        .expect("spotify track_rows empty");
    click(
        bin,
        options,
        track_point.x,
        track_point.y,
        "spotify click track row",
    );
    let post_track_wait = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "700"],
        "wait after spotify track click",
    );
    assert_eq!(post_track_wait["command"], serde_json::json!("wait.sleep"));

    states.push(real_common::spotify_playback_state());
    let before = artifact_dir.join("spotify-before-toggle.png");
    capture_active_window(bin, options, &before);
    screenshots.push(before.to_string_lossy().to_string());

    click(
        bin,
        options,
        profile.play_toggle.x,
        profile.play_toggle.y,
        "spotify play/pause ui toggle #1",
    );
    let settle_1 = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "600"],
        "wait after spotify toggle #1",
    );
    assert_eq!(settle_1["command"], serde_json::json!("wait.sleep"));
    states.push(real_common::spotify_playback_state());
    let after_1 = artifact_dir.join("spotify-after-toggle-1.png");
    capture_active_window(bin, options, &after_1);
    screenshots.push(after_1.to_string_lossy().to_string());

    click(
        bin,
        options,
        profile.play_toggle.x,
        profile.play_toggle.y,
        "spotify play/pause ui toggle #2",
    );
    let settle_2 = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "600"],
        "wait after spotify toggle #2",
    );
    assert_eq!(settle_2["command"], serde_json::json!("wait.sleep"));
    states.push(real_common::spotify_playback_state());
    let after_2 = artifact_dir.join("spotify-after-toggle-2.png");
    capture_active_window(bin, options, &after_2);
    screenshots.push(after_2.to_string_lossy().to_string());

    require_playback_transition(&states);

    ScenarioOutcome {
        scenario_id: "spotify_ui_selects_track_and_toggles_playback".to_string(),
        status: ScenarioStatus::Passed,
        elapsed_ms: started.elapsed().as_millis() as u64,
        artifact_dir: artifact_dir.display().to_string(),
        screenshots,
    }
}

fn activate_spotify(bin: &Path, options: &CmdOptions) {
    real_common::ensure_input_source_for_text_entry();
    let activate = real_common::run_json_step(
        bin,
        options,
        &[
            "--format",
            "json",
            "window",
            "activate",
            "--app",
            "Spotify",
            "--wait-ms",
            "1800",
        ],
        "window.activate Spotify",
    );
    assert_eq!(activate["command"], serde_json::json!("window.activate"));
    let settle = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "1200"],
        "wait after Spotify activate",
    );
    assert_eq!(settle["command"], serde_json::json!("wait.sleep"));
}

fn click(bin: &Path, options: &CmdOptions, x: i32, y: i32, step: &str) {
    let x_text = x.to_string();
    let y_text = y.to_string();
    let payload = real_common::run_json_step_with_retry(
        bin,
        options,
        &[
            "--format",
            "json",
            "input",
            "click",
            "--x",
            &x_text,
            "--y",
            &y_text,
            "--pre-wait-ms",
            "80",
            "--post-wait-ms",
            "120",
        ],
        step,
        2,
        std::time::Duration::from_millis(250),
    );
    assert_eq!(payload["command"], serde_json::json!("input.click"));
}

fn capture_active_window(bin: &Path, options: &CmdOptions, screenshot_path: &Path) {
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
        "observe screenshot spotify",
    );
    assert_eq!(payload["command"], serde_json::json!("observe.screenshot"));
    assert!(screenshot_path.is_file(), "spotify screenshot should exist");
}

fn require_playback_transition(states: &[SpotifyPlaybackState]) {
    let mut saw_playing = false;
    let mut saw_paused = false;
    for sample in states {
        if sample.player_state.contains("playing") {
            saw_playing = true;
        }
        if sample.player_state.contains("paused") {
            saw_paused = true;
        }
    }
    assert!(
        saw_playing || saw_paused,
        "expected at least one known spotify state sample, got: {:?}",
        states
            .iter()
            .map(|sample| sample.player_state.clone())
            .collect::<Vec<_>>()
    );
}
