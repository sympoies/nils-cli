#[path = "integration/board.rs"]
mod board;
#[path = "integration/broker_retirement.rs"]
mod broker_retirement;
#[path = "integration/cli.rs"]
mod cli;
#[path = "integration/cli_launch_env.rs"]
mod cli_launch_env;
#[path = "integration/coordination.rs"]
mod coordination;
#[path = "integration/coordination_server.rs"]
mod coordination_server;
#[path = "integration/diagnose.rs"]
mod diagnose;
#[path = "integration/lineage.rs"]
mod lineage;
#[path = "integration/metadata.rs"]
mod metadata;
#[path = "integration/readiness.rs"]
mod readiness;
#[path = "integration/resume_mail.rs"]
mod resume_mail;
#[path = "integration/retitle_v3.rs"]
mod retitle_v3;
#[path = "integration/work_context_admission.rs"]
mod work_context_admission;

/// Provider binaries an integration run resolves when a test pins none. The path
/// cannot exist, so no test reaches an installed provider CLI through `PATH`.
const UNPINNED_PROVIDER_BIN: &str = "/nonexistent/agent-session-unpinned-test-agent";

/// Command options for the `agent-session` binary with provider binaries pinned
/// away from `PATH`; a test's own environment entries override these defaults.
fn hermetic_provider_options(dir: &std::path::Path) -> nils_test_support::cmd::CmdOptions {
    nils_test_support::cmd::CmdOptions::new()
        .with_cwd(dir)
        .with_env("AGENT_SESSION_CODEX_BIN", UNPINNED_PROVIDER_BIN)
        .with_env("AGENT_SESSION_CLAUDE_BIN", UNPINNED_PROVIDER_BIN)
}
