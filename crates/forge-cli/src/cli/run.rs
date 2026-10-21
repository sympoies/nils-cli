use clap::{Args, Subcommand};

#[derive(Args, Debug)]
pub struct RunArgs {
    #[command(subcommand)]
    pub command: RunCommand,
}
#[derive(Subcommand, Debug)]
pub enum RunCommand {
    /// Request exactly one rerun, then read its attempt and job receipt.
    Rerun(RunRerunArgs),
}
#[derive(Args, Debug)]
pub struct RunRerunArgs {
    /// Existing workflow run database id.
    pub run_id: u64,
    /// Rerun this job and its dependent jobs; must belong to this run attempt.
    #[arg(long, conflicts_with = "failed_only")]
    pub job: Option<u64>,
    /// Rerun only failed jobs and their dependent jobs.
    #[arg(long)]
    pub failed_only: bool,
    /// Exact full run SHA, also checked against each associated current PR head.
    #[arg(long)]
    pub expected_head: String,
    /// Bound receipt reads after the single write; zero performs one receipt probe.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(0..=60))]
    pub receipt_timeout_secs: u64,
}
