//! Scope the wait budget before identity, repository and configuration setup.
use std::cell::Cell;
use std::ffi::OsString;
use std::io;
use std::process::{Command as ProcessCommand, Output};
use std::time::{Duration, Instant};

use crate::backend::{ProcessOutputError, output_with_limits};
use crate::cli::{self, Command, PrArgs, PrCommand};
use crate::error::ForgeError;

#[derive(Clone, Copy)]
struct Budget {
    start: Instant,
    deadline: Instant,
}
thread_local! {
    static ACTIVE: Cell<Option<Budget>> = const { Cell::new(None) };
}

struct Scope(Option<Budget>);
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(self.0));
    }
}

/// Preserve the public dispatcher while activating only a live wait command.
pub fn dispatch(args: Vec<OsString>) -> i32 {
    let start = Instant::now();
    let cli = match cli::parse_or_exit(args) {
        Ok(cli) => cli,
        Err(code) => return code,
    };
    let budget = match &cli.command {
        Some(Command::Pr(PrArgs {
            command: Some(PrCommand::WaitChecks(args)),
        })) if !cli.dry_run => Some(Budget {
            start,
            deadline: start + args.timeout,
        }),
        _ => None,
    };
    let _scope = Scope(ACTIVE.with(|active| active.replace(budget)));
    cli::dispatch_parsed(cli)
}

pub(super) fn start() -> Option<Instant> {
    ACTIVE.with(|active| active.get().map(|budget| budget.start))
}
pub(super) fn deadline() -> Option<Instant> {
    ACTIVE.with(|active| active.get().map(|budget| budget.deadline))
}

pub(crate) fn check() -> Result<(), ForgeError> {
    if deadline().is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(ForgeError::unavailable(
            super::pr_checks::schema_err(),
            "checks_timeout",
            "deadline reached during repository/configuration discovery",
            None,
        ));
    }
    Ok(())
}

/// Ordinary commands retain their existing unbounded optional Git lookup.
/// A wait command passes only its remaining budget, including capture cleanup.
pub(crate) fn output(command: &mut ProcessCommand) -> io::Result<Output> {
    let Some(deadline) = deadline() else {
        return command.output();
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining == Duration::ZERO {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "wait budget expired",
        ));
    }
    match output_with_limits(command, Some(remaining), 8 * 1024 * 1024) {
        Ok(output) if Instant::now() < deadline => Ok(output),
        Ok(_) | Err(ProcessOutputError::Timeout { .. }) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "wait discovery timed out",
        )),
        Err(ProcessOutputError::Io(error)) => Err(error),
        Err(ProcessOutputError::OutputLimit { .. }) => {
            Err(io::Error::other("Git discovery output limit"))
        }
    }
}
