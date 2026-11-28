use super::OutputFormat;
use clap::{Args, ValueHint};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// Delete only a positively verified stopped session; refuses running, working, or
    /// unknown state and never stops the runtime. Requires --expected-incarnation.
    #[arg(
        long,
        group = "delete_condition",
        requires = "expected_incarnation",
        conflicts_with_all = ["if_idle", "leave_processes", "recover_split_cgroup"]
    )]
    pub if_stopped: bool,

    /// Retire a running managed Claude or Codex session only when provider idleness and an
    /// empty composer are verified. Claude retirement is best effort: it refuses while a tmux
    /// client is attached, but a client that attaches and types between the final check and
    /// teardown can still lose input. Requires --expected-incarnation.
    #[arg(
        long,
        group = "delete_condition",
        requires = "expected_incarnation",
        conflicts_with_all = ["if_stopped", "leave_processes", "recover_split_cgroup"]
    )]
    pub if_idle: bool,

    /// Refuse deletion if the session runtime incarnation has changed.
    #[arg(long, requires = "delete_condition")]
    pub expected_incarnation: Option<String>,

    /// Also require zero unread messages, checked atomically with deletion.
    /// Refuses unread or unknown mailbox state. Requires --if-stopped or --if-idle.
    #[arg(long, requires = "delete_condition")]
    pub if_unread_empty: bool,

    /// Stop only the verified tmux target and delete state, leaving processes running.
    #[arg(long, conflicts_with_all = ["if_stopped", "if_idle", "recover_split_cgroup"])]
    pub leave_processes: bool,

    /// Explicitly recover a split Linux process-session/cgroup boundary using
    /// captured PID/start-time identities and per-process identity checks.
    #[arg(long, conflicts_with_all = ["if_stopped", "if_idle", "leave_processes"])]
    pub recover_split_cgroup: bool,

    /// Session id.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Delete even though sessions on this machine still name this one as
    /// their parent; they are left as orphans.
    #[arg(long)]
    pub orphan_children: bool,

    /// tmux binary override.
    #[arg(long = "tmux-bin", value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub tmux_bin: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}
