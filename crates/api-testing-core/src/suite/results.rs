use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuiteRunSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuiteCaseResult {
    pub id: String,
    #[serde(rename = "type")]
    pub case_type: String,
    pub status: String,
    pub duration_ms: u64,
    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub assertions: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_file: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuiteRunResults {
    pub version: u32,
    pub suite: String,
    pub suite_file: String,
    pub run_id: String,
    pub started_at: String,
    pub finished_at: String,
    pub output_dir: String,
    pub summary: SuiteRunSummary,
    pub cases: Vec<SuiteCaseResult>,
}

impl SuiteRunResults {
    pub fn exit_code(&self) -> i32 {
        if self.summary.failed > 0 {
            2
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn suite_results_exit_code_reflects_failures() {
        let ok = SuiteRunResults {
            version: 1,
            suite: "suite".to_string(),
            suite_file: "suite.json".to_string(),
            run_id: "run-1".to_string(),
            started_at: "start".to_string(),
            finished_at: "end".to_string(),
            output_dir: "out".to_string(),
            summary: SuiteRunSummary {
                total: 1,
                passed: 1,
                failed: 0,
                skipped: 0,
            },
            cases: Vec::new(),
        };
        assert_eq!(ok.exit_code(), 0);

        let failed = SuiteRunResults {
            summary: SuiteRunSummary {
                total: 1,
                passed: 0,
                failed: 1,
                skipped: 0,
            },
            ..ok
        };
        assert_eq!(failed.exit_code(), 2);
    }
}
