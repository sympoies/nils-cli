use serde::Serialize;

pub const BUNDLE_MANIFEST_SCHEMA_VERSION: &str = "agentctl.debug.bundle.v1";
pub const BUNDLE_MANIFEST_VERSION: u32 = 1;
pub const BUNDLE_MANIFEST_FILE_NAME: &str = "manifest.json";
pub const BUNDLE_ARTIFACTS_DIR: &str = "artifacts";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactStatus {
    Collected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BundleArtifact {
    pub id: String,
    pub path: String,
    pub status: ArtifactStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BundleArtifact {
    pub fn collected(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            status: ArtifactStatus::Collected,
            error: None,
        }
    }

    pub fn failed(
        id: impl Into<String>,
        path: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            status: ArtifactStatus::Failed,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BundleSummary {
    pub total_artifacts: usize,
    pub collected: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BundleManifest {
    pub schema_version: &'static str,
    pub manifest_version: u32,
    pub command: &'static str,
    pub output_dir: String,
    pub partial_failure: bool,
    pub summary: BundleSummary,
    pub artifacts: Vec<BundleArtifact>,
}

impl BundleManifest {
    pub fn from_artifacts(output_dir: String, artifacts: Vec<BundleArtifact>) -> Self {
        let failed = artifacts
            .iter()
            .filter(|artifact| artifact.status == ArtifactStatus::Failed)
            .count();
        let collected = artifacts.len().saturating_sub(failed);

        Self {
            schema_version: BUNDLE_MANIFEST_SCHEMA_VERSION,
            manifest_version: BUNDLE_MANIFEST_VERSION,
            command: "debug.bundle",
            output_dir,
            partial_failure: failed > 0,
            summary: BundleSummary {
                total_artifacts: artifacts.len(),
                collected,
                failed,
            },
            artifacts,
        }
    }
}
