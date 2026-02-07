use std::path::Path;
use std::time::Instant;

use nils_test_support::cmd::CmdOptions;

use crate::real_apps::arc_navigation::{click as arc_click, ArcYoutubeProfile};
use crate::real_apps::matrix::{ScenarioOutcome, ScenarioStatus};
use crate::real_common::{self, SpotifyPlaybackState};

pub fn cross_app_arc_spotify_focus_and_state_recovery(
    bin: &Path,
    options: &CmdOptions,
    artifact_dir: &Path,
) -> ScenarioOutcome {
    let started = Instant::now();
    real_common::require_app_installed("Arc");
    real_common::require_app_installed("Spotify");
    let _arc_cleanup_guard = real_common::ArcCleanupGuard::new();
    let _spotify_cleanup_guard = real_common::SpotifyPlaybackCleanupGuard::capture();

    let profile = real_common::load_profile();
    let arc_profile = ArcYoutubeProfile::from_default_profile();
    let mut screenshots = Vec::new();

    activate_app(bin, options, "Spotify");
    let before = real_common::spotify_playback_state();

    activate_app(bin, options, "Arc");
    let arc_click_point = profile.arc.player_focus;
    arc_click(bin, options, &arc_click_point, "cross-app arc focus click");
    let arc_shot = artifact_dir.join("cross-app-arc.png");
    capture_active_window(bin, options, &arc_shot);
    screenshots.push(arc_shot.to_string_lossy().to_string());

    activate_app(bin, options, "Spotify");
    let after = real_common::spotify_playback_state();
    let spotify_shot = artifact_dir.join("cross-app-spotify-return.png");
    capture_active_window(bin, options, &spotify_shot);
    screenshots.push(spotify_shot.to_string_lossy().to_string());

    assert!(
        !after.track_name.is_empty() || !before.track_name.is_empty(),
        "expected spotify to expose track metadata before/after cross-app switch"
    );

    let state_payload = serde_json::json!({
        "before": spotify_state_json(&before),
        "after": spotify_state_json(&after),
        "arc_profile_app": arc_profile.app_name,
    });
    real_common::write_json(
        &artifact_dir.join("cross-app-spotify-states.json"),
        &state_payload,
    );

    ScenarioOutcome {
        scenario_id: "cross_app_arc_spotify_focus_and_state_recovery".to_string(),
        status: ScenarioStatus::Passed,
        elapsed_ms: started.elapsed().as_millis() as u64,
        artifact_dir: artifact_dir.display().to_string(),
        screenshots,
    }
}

fn activate_app(bin: &Path, options: &CmdOptions, app: &str) {
    let activate = real_common::run_json_step(
        bin,
        options,
        &[
            "--format",
            "json",
            "window",
            "activate",
            "--app",
            app,
            "--wait-ms",
            "1800",
        ],
        &format!("window.activate {app}"),
    );
    assert_eq!(activate["command"], serde_json::json!("window.activate"));
    let settle = real_common::run_json_step(
        bin,
        options,
        &["--format", "json", "wait", "sleep", "--ms", "1200"],
        &format!("wait after activate {app}"),
    );
    assert_eq!(settle["command"], serde_json::json!("wait.sleep"));
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
        "observe screenshot cross-app",
    );
    assert_eq!(payload["command"], serde_json::json!("observe.screenshot"));
    assert!(
        screenshot_path.is_file(),
        "cross-app screenshot should exist"
    );
}

fn spotify_state_json(sample: &SpotifyPlaybackState) -> serde_json::Value {
    serde_json::json!({
        "player_state": sample.player_state,
        "track_name": sample.track_name,
        "artist": sample.artist,
    })
}
