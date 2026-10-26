#[path = "integration/admission.rs"]
mod admission;
#[path = "integration/board.rs"]
mod board;
#[path = "integration/broker_retirement.rs"]
mod broker_retirement;
#[path = "integration/claude_doctor.rs"]
mod claude_doctor;
#[path = "integration/cli.rs"]
mod cli;
#[path = "integration/cli_launch_env.rs"]
mod cli_launch_env;
#[path = "integration/codex_notify.rs"]
mod codex_notify;
#[path = "integration/coordination.rs"]
mod coordination;
#[path = "integration/coordination_server.rs"]
mod coordination_server;
#[path = "integration/delete_guards.rs"]
mod delete_guards;
#[path = "integration/diagnose.rs"]
mod diagnose;
#[path = "integration/doctor.rs"]
mod doctor;
#[path = "integration/lineage.rs"]
mod lineage;
#[path = "integration/mailbox_any_unread.rs"]
mod mailbox_any_unread;
#[path = "integration/mailbox_authority.rs"]
mod mailbox_authority;
#[path = "integration/mailbox_burst.rs"]
mod mailbox_burst;
#[path = "integration/mailbox_expiry.rs"]
mod mailbox_expiry;
#[path = "integration/mailbox_quota_warning.rs"]
mod mailbox_quota_warning;
#[path = "integration/metadata.rs"]
mod metadata;
#[path = "integration/readiness.rs"]
mod readiness;
#[path = "integration/resume_contention.rs"]
mod resume_contention;
#[path = "integration/resume_mail.rs"]
mod resume_mail;
#[path = "integration/resume_status.rs"]
mod resume_status;
#[path = "integration/retitle_v3.rs"]
mod retitle_v3;
#[path = "integration/tmp_sweep.rs"]
mod tmp_sweep;
#[path = "integration/work_context_admission.rs"]
mod work_context_admission;

/// Provider binaries an integration run resolves when a test pins none. The path
/// cannot exist, so no test reaches an installed provider CLI through `PATH`.
const UNPINNED_PROVIDER_BIN: &str = "/nonexistent/agent-session-unpinned-test-agent";

/// Command options for the `agent-session` binary with provider binaries pinned
/// away from `PATH`; host provider config overrides are removed unless a test
/// explicitly owns one in its environment entries.
fn hermetic_provider_options(dir: &std::path::Path) -> nils_test_support::cmd::CmdOptions {
    nils_test_support::cmd::CmdOptions::new()
        .with_cwd(dir)
        .with_env_remove_many(&["CLAUDE_CONFIG_DIR", "CODEX_HOME"])
        // Per-session TMPDIR directories land under AGENT_HOME; keep them in the test directory.
        .with_env("AGENT_HOME", &dir.join("agent-home").to_string_lossy())
        .with_env("AGENT_SESSION_CODEX_BIN", UNPINNED_PROVIDER_BIN)
        .with_env("AGENT_SESSION_CLAUDE_BIN", UNPINNED_PROVIDER_BIN)
}

#[path = "integration/origin.rs"]
mod origin;

#[path = "integration/mailbox_reminder.rs"]
mod mailbox_reminder;

#[path = "integration/delete_escaped.rs"]
mod delete_escaped;

#[path = "integration/broker_proof.rs"]
mod broker_proof;

#[path = "integration/notice_start.rs"]
mod notice_start;
