use std::path::Path;

use crate::suite::auth::SuiteAuthManager;
use crate::suite::schema::{SuiteCleanup, SuiteDefaults};

pub struct CleanupContext<'a> {
    pub repo_root: &'a Path,
    pub run_dir: &'a Path,
    pub case_id: &'a str,
    pub safe_id: &'a str,

    pub main_response_file: Option<&'a Path>,
    pub main_stderr_file: &'a Path,

    pub allow_writes_flag: bool,
    pub effective_env: &'a str,
    pub effective_no_history: bool,

    pub suite_defaults: &'a SuiteDefaults,
    pub env_rest_url: &'a str,
    pub env_gql_url: &'a str,

    pub rest_config_dir: &'a str,
    pub rest_url: &'a str,
    pub rest_token: &'a str,

    pub gql_config_dir: &'a str,
    pub gql_url: &'a str,
    pub gql_jwt: &'a str,

    pub access_token_for_case: &'a str,
    pub auth_manager: Option<&'a mut SuiteAuthManager>,

    pub cleanup: Option<&'a SuiteCleanup>,
}
