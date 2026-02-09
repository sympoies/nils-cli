use super::super::bundle::collect_command_artifact;
use super::super::schema::BundleArtifact;
use std::path::Path;

pub const ARTIFACT_ID: &str = "screen-record-preflight";
pub const ARTIFACT_RELATIVE_PATH: &str = "artifacts/30-screen-record-preflight.json";

pub fn collect(output_dir: &Path) -> BundleArtifact {
    collect_command_artifact(
        output_dir,
        ARTIFACT_ID,
        ARTIFACT_RELATIVE_PATH,
        "screen-record",
        &["--preflight"],
    )
}
