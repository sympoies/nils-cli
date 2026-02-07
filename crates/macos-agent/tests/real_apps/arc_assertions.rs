use std::path::Path;
use std::time::Instant;

use nils_test_support::cmd::CmdOptions;

use crate::real_apps::arc_navigation::{
    activate_arc, capture_active_window, click, open_youtube_home, wait_for_arc, ArcYoutubeProfile,
};
use crate::real_apps::matrix::{ScenarioOutcome, ScenarioStatus};
use crate::real_common;

pub fn arc_youtube_play_pause_and_comment_checkpoint(
    bin: &Path,
    options: &CmdOptions,
    artifact_dir: &Path,
    profile: &ArcYoutubeProfile,
    selected_video_indices: &[usize],
) -> ScenarioOutcome {
    let started = Instant::now();
    let _cleanup_guard = real_common::ArcCleanupGuard::new();
    assert!(
        !selected_video_indices.is_empty(),
        "selected_video_indices cannot be empty"
    );

    real_common::require_app_installed(&profile.app_name);
    let mut screenshots = Vec::new();
    let common_profile = real_common::load_profile();
    let player_focus = common_profile.arc.player_focus;
    let comment_checkpoint = common_profile.arc.comment_checkpoint;

    for index in selected_video_indices {
        let point = profile
            .video_tiles
            .get(*index)
            .unwrap_or_else(|| panic!("selected video index {index} out of range"));
        activate_arc(bin, options, &profile.app_name);
        open_youtube_home(bin, options, &profile.app_name, &profile.youtube_home_url);
        click(
            bin,
            options,
            point,
            &format!("arc select video index {index}"),
        );
        wait_for_arc(bin, options, &profile.app_name);

        click(
            bin,
            options,
            &player_focus,
            "arc focus player for play/pause",
        );
        real_common::send_hotkey(bin, options, None, "space", "arc toggle play/pause #1");
        let settle_1 = real_common::run_json_step(
            bin,
            options,
            &["--format", "json", "wait", "sleep", "--ms", "450"],
            "wait after play/pause #1",
        );
        assert_eq!(settle_1["command"], serde_json::json!("wait.sleep"));

        real_common::send_hotkey(bin, options, None, "space", "arc toggle play/pause #2");
        let settle_2 = real_common::run_json_step(
            bin,
            options,
            &["--format", "json", "wait", "sleep", "--ms", "450"],
            "wait after play/pause #2",
        );
        assert_eq!(settle_2["command"], serde_json::json!("wait.sleep"));

        click(
            bin,
            options,
            &comment_checkpoint,
            "arc click comment checkpoint",
        );
        let settle_comment = real_common::run_json_step(
            bin,
            options,
            &["--format", "json", "wait", "sleep", "--ms", "700"],
            "wait after comment checkpoint click",
        );
        assert_eq!(settle_comment["command"], serde_json::json!("wait.sleep"));
        wait_for_arc(bin, options, &profile.app_name);

        let playback_screenshot = artifact_dir.join(format!("arc-video-{index}-playback.png"));
        capture_active_window(bin, options, &playback_screenshot);
        screenshots.push(playback_screenshot.to_string_lossy().to_string());

        let comment_screenshot = artifact_dir.join(format!("arc-video-{index}-comment.png"));
        capture_active_window(bin, options, &comment_screenshot);
        screenshots.push(comment_screenshot.to_string_lossy().to_string());
    }

    ScenarioOutcome {
        scenario_id: "arc_youtube_play_pause_and_comment_checkpoint".to_string(),
        status: ScenarioStatus::Passed,
        elapsed_ms: started.elapsed().as_millis() as u64,
        artifact_dir: artifact_dir.display().to_string(),
        screenshots,
    }
}
