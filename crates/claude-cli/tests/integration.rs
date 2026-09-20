// Consolidated integration test target.
// Each behavioural area is declared as a submodule here so the crate
// links one integration test binary instead of many. This keeps the
// dev-loop link phase O(crates) instead of O(test-files).

#[path = "integration/support.rs"]
mod support;

#[path = "integration/agent_commit.rs"]
mod agent_commit;
#[path = "integration/agent_doctor.rs"]
mod agent_doctor;
#[path = "integration/agent_oneshot.rs"]
mod agent_oneshot;
#[path = "integration/agent_resume.rs"]
mod agent_resume;
#[path = "integration/auth.rs"]
mod auth;
#[path = "integration/completion_contract.rs"]
mod completion_contract;
#[path = "integration/completion_flags_contract.rs"]
mod completion_flags_contract;
#[path = "integration/config.rs"]
mod config;
#[path = "integration/fixtures.rs"]
mod fixtures;
#[path = "integration/main_entrypoint.rs"]
mod main_entrypoint;
#[path = "integration/prompt_segment.rs"]
mod prompt_segment;
#[path = "integration/usage.rs"]
mod usage;
