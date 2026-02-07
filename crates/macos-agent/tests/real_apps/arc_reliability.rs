use std::path::Path;
use std::time::Duration;

use nils_test_support::cmd::CmdOptions;

use crate::real_apps::arc_assertions::arc_youtube_play_pause_and_comment_checkpoint;
use crate::real_apps::arc_navigation::{
    arc_youtube_opens_home_and_clicks_three_tiles, ArcYoutubeProfile,
};
use crate::real_apps::matrix::{ScenarioOutcome, ScenarioStatus};
use crate::real_common;

pub fn arc_youtube_multi_video_play_pause_and_comments(
    bin: &Path,
    options: &CmdOptions,
    artifact_dir: &Path,
    profile: &ArcYoutubeProfile,
) -> ScenarioOutcome {
    let open_and_click = real_common::run_idempotent_with_retry(
        "arc_open_and_click_three_tiles",
        2,
        Duration::from_millis(300),
        || arc_youtube_opens_home_and_clicks_three_tiles(bin, options, artifact_dir, profile),
    );

    let playback_and_comment = real_common::run_idempotent_with_retry(
        "arc_play_pause_and_comment",
        2,
        Duration::from_millis(300),
        || {
            arc_youtube_play_pause_and_comment_checkpoint(
                bin,
                options,
                artifact_dir,
                profile,
                &[0, 1, 2],
            )
        },
    );

    let mut screenshots = open_and_click.screenshots;
    screenshots.extend(playback_and_comment.screenshots);

    let elapsed = open_and_click.elapsed_ms + playback_and_comment.elapsed_ms;
    ScenarioOutcome {
        scenario_id: "arc_youtube_multi_video_play_pause_and_comments".to_string(),
        status: ScenarioStatus::Passed,
        elapsed_ms: elapsed,
        artifact_dir: artifact_dir.display().to_string(),
        screenshots,
    }
}
