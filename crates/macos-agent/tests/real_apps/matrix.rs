use std::collections::HashSet;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const SUPPORTED_APPS: [&str; 3] = ["arc", "spotify", "finder"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioOutcome {
    pub scenario_id: String,
    pub status: ScenarioStatus,
    pub elapsed_ms: u64,
    pub artifact_dir: String,
    #[serde(default)]
    pub screenshots: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioBucketSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    #[serde(default)]
    pub scenario_ids: Vec<String>,
}

impl ScenarioBucketSummary {
    fn push(&mut self, scenario: &ScenarioOutcome) {
        self.total += 1;
        match scenario.status {
            ScenarioStatus::Passed => self.passed += 1,
            ScenarioStatus::Failed => self.failed += 1,
            ScenarioStatus::Skipped => self.skipped += 1,
        }
        self.scenario_ids.push(scenario.scenario_id.clone());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixSummary {
    pub base: ScenarioBucketSummary,
    pub extended: ScenarioBucketSummary,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactIndex {
    #[serde(default)]
    pub scenarios: Vec<ScenarioOutcome>,
    pub summary: MatrixSummary,
}

pub fn selected_apps_from_env(raw: Option<&str>) -> Vec<&'static str> {
    let Some(raw) = raw else {
        return SUPPORTED_APPS.to_vec();
    };

    let selected: HashSet<String> = raw
        .split(',')
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect();

    if selected.is_empty() {
        return SUPPORTED_APPS.to_vec();
    }

    SUPPORTED_APPS
        .iter()
        .copied()
        .filter(|app| selected.contains(*app))
        .collect()
}

pub fn write_artifact_index(
    path: &Path,
    scenarios: &[ScenarioOutcome],
) -> io::Result<ArtifactIndex> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let index = ArtifactIndex {
        scenarios: scenarios.to_vec(),
        summary: classify_base_vs_extended(scenarios),
    };
    let payload = serde_json::to_vec_pretty(&index)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    std::fs::write(path, payload)?;
    Ok(index)
}

pub fn classify_base_vs_extended(scenarios: &[ScenarioOutcome]) -> MatrixSummary {
    let mut summary = MatrixSummary::default();
    for scenario in scenarios {
        if is_extended_scenario_id(&scenario.scenario_id) {
            summary.extended.push(scenario);
        } else {
            summary.base.push(scenario);
        }
    }
    summary
}

pub fn subset_selection_matches(raw: Option<&str>, expected: &[&str]) -> bool {
    selected_apps_from_env(raw) == expected
}

pub fn artifact_index_has_required_fields(index: &ArtifactIndex) -> bool {
    let Ok(value) = serde_json::to_value(index) else {
        return false;
    };
    let Some(rows) = value.get("scenarios").and_then(serde_json::Value::as_array) else {
        return false;
    };

    rows.iter().all(|row| {
        row.get("scenario_id")
            .and_then(serde_json::Value::as_str)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
            && row
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some()
            && row
                .get("elapsed_ms")
                .and_then(serde_json::Value::as_u64)
                .is_some()
            && row
                .get("artifact_dir")
                .and_then(serde_json::Value::as_str)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
            && row
                .get("screenshots")
                .and_then(serde_json::Value::as_array)
                .is_some()
    })
}

pub fn summary_has_base_extended_separation(summary: &MatrixSummary) -> bool {
    let base_ids: HashSet<&str> = summary
        .base
        .scenario_ids
        .iter()
        .map(String::as_str)
        .collect();
    let extended_ids: HashSet<&str> = summary
        .extended
        .scenario_ids
        .iter()
        .map(String::as_str)
        .collect();

    base_ids.is_disjoint(&extended_ids)
        && summary.base.total == summary.base.scenario_ids.len()
        && summary.extended.total == summary.extended.scenario_ids.len()
}

fn is_extended_scenario_id(scenario_id: &str) -> bool {
    let normalized = scenario_id.trim().to_ascii_lowercase();
    normalized.starts_with("cross_app_")
        || normalized.contains("_extended")
        || normalized.contains("extended_")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        artifact_index_has_required_fields, classify_base_vs_extended, selected_apps_from_env,
        subset_selection_matches, summary_has_base_extended_separation, write_artifact_index,
        ScenarioOutcome, ScenarioStatus, SUPPORTED_APPS,
    };

    #[test]
    fn app_subset_selection_defaults_to_all() {
        assert_eq!(selected_apps_from_env(None), SUPPORTED_APPS);
        assert_eq!(selected_apps_from_env(Some("  ")), SUPPORTED_APPS);
    }

    #[test]
    fn app_subset_selection_is_deterministic() {
        let selected = selected_apps_from_env(Some("finder,arc,arc,spotify"));
        assert_eq!(selected, vec!["arc", "spotify", "finder"]);
        assert!(subset_selection_matches(
            Some("spotify,finder,arc"),
            &["arc", "spotify", "finder"]
        ));
    }

    #[test]
    fn app_subset_selection_filters_unsupported_apps() {
        let selected = selected_apps_from_env(Some("mail,spotify,notes"));
        assert_eq!(selected, vec!["spotify"]);
    }

    #[test]
    fn write_artifact_index_includes_required_fields() {
        let dir = tempdir().expect("tempdir");
        let index_path = dir.path().join("artifacts/index.json");
        let scenarios = vec![ScenarioOutcome {
            scenario_id: "finder_navigation_and_state_checks".to_string(),
            status: ScenarioStatus::Passed,
            elapsed_ms: 1250,
            artifact_dir: "/tmp/finder".to_string(),
            screenshots: vec!["/tmp/finder/step-1.png".to_string()],
        }];

        let index = write_artifact_index(&index_path, &scenarios).expect("write index");
        assert!(index_path.is_file());
        assert!(artifact_index_has_required_fields(&index));

        let persisted = fs::read_to_string(&index_path).expect("read index");
        let json: serde_json::Value = serde_json::from_str(&persisted).expect("json parse");
        assert!(json["scenarios"].is_array());
        assert_eq!(
            json["scenarios"][0]["scenario_id"],
            "finder_navigation_and_state_checks"
        );
        assert_eq!(json["scenarios"][0]["status"], "passed");
    }

    #[test]
    fn classify_base_vs_extended_separates_summary_buckets() {
        let scenarios = vec![
            ScenarioOutcome {
                scenario_id: "finder_navigation_and_state_checks".to_string(),
                status: ScenarioStatus::Passed,
                elapsed_ms: 1200,
                artifact_dir: "/tmp/finder".to_string(),
                screenshots: vec![],
            },
            ScenarioOutcome {
                scenario_id: "cross_app_arc_spotify_focus_and_state_recovery".to_string(),
                status: ScenarioStatus::Failed,
                elapsed_ms: 2400,
                artifact_dir: "/tmp/cross-app".to_string(),
                screenshots: vec![],
            },
        ];

        let summary = classify_base_vs_extended(&scenarios);
        assert_eq!(summary.base.total, 1);
        assert_eq!(summary.extended.total, 1);
        assert_eq!(summary.base.passed, 1);
        assert_eq!(summary.extended.failed, 1);
        assert!(summary_has_base_extended_separation(&summary));
    }
}
