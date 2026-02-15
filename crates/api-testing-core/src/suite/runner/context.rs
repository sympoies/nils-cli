use std::collections::HashSet;
use std::path::PathBuf;

use nils_term::progress::Progress;

use crate::suite::results::SuiteRunResults;
use crate::suite::schema::LoadedSuite;

#[derive(Debug, Clone)]
pub struct SuiteRunOptions {
    pub required_tags: Vec<String>,
    pub only_ids: HashSet<String>,
    pub skip_ids: HashSet<String>,
    pub allow_writes_flag: bool,
    pub fail_fast: bool,
    pub output_dir_base: PathBuf,
    pub env_rest_url: String,
    pub env_gql_url: String,
    pub env_grpc_url: String,
    pub progress: Option<Progress>,
}

#[derive(Debug, Clone)]
pub struct SuiteRunOutput {
    pub run_dir_abs: PathBuf,
    pub results: SuiteRunResults,
}

pub(super) fn suite_display_name(loaded: &LoadedSuite) -> String {
    let name = loaded.manifest.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    loaded
        .suite_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("suite")
        .to_string()
}

pub(super) fn case_type_normalized(case_type_raw: &str) -> String {
    case_type_raw.trim().to_ascii_lowercase()
}

pub(super) fn default_rest_flow_token_jq() -> String {
    ".. | objects | (.accessToken? // .access_token? // .token? // empty) | select(type==\"string\" and length>0) | .".to_string()
}
