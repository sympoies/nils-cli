mod command;
mod git;
pub(crate) mod install;
mod options;
pub(crate) mod preflight;
mod receipt;
mod transaction;

pub(crate) use command::run;
#[cfg(test)]
pub(crate) use options::{OptionArity, option_contract};
pub(crate) use options::{OptionKind, clap_command, option_for_spelling};
