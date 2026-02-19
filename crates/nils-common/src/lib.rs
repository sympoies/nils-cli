//! # nils-common public API contracts (Task 1.2)
//!
//! This crate keeps runtime behavior stable. The items below are contract specifications for
//! planned cross-CLI modules; this step is documentation-only and does not implement/export all
//! modules yet.
//!
//! ## Planned module contracts (spec only)
//!
//! ```text
//! // env
//! pub fn is_truthy(input: &str) -> bool;
//! pub fn env_truthy(name: &str) -> bool;
//! pub fn env_truthy_or(name: &str, default: bool) -> bool;
//! pub fn env_or_default(name: &str, default: &str) -> String;
//! pub fn no_color_enabled() -> bool;
//!
//! // shell
//! pub enum AnsiStripMode { CsiSgrOnly, CsiAnyTerminator }
//! pub fn quote_posix_single(input: &str) -> String;
//! pub fn strip_ansi(input: &str, mode: AnsiStripMode) -> std::borrow::Cow<'_, str>;
//!
//! // process (expansion on top of existing path lookup helpers)
//! pub struct ProcessOutput {
//!   pub status: std::process::ExitStatus,
//!   pub stdout: Vec<u8>,
//!   pub stderr: Vec<u8>,
//! }
//! pub enum ProcessError { Io(std::io::Error), NonZero(ProcessOutput) }
//! pub fn run_output(program: &str, args: &[&str]) -> Result<ProcessOutput, ProcessError>;
//! pub fn run_checked(program: &str, args: &[&str]) -> Result<(), ProcessError>;
//! pub fn run_stdout_trimmed(program: &str, args: &[&str]) -> Result<String, ProcessError>;
//!
//! // git
//! pub fn is_inside_work_tree(cwd: &std::path::Path) -> Result<bool, ProcessError>;
//! pub fn repo_root(cwd: &std::path::Path) -> Result<Option<std::path::PathBuf>, ProcessError>;
//! pub fn rev_parse(cwd: &std::path::Path, args: &[&str]) -> Result<String, ProcessError>;
//! pub fn rev_parse_opt(cwd: &std::path::Path, args: &[&str]) -> Result<Option<String>, ProcessError>;
//!
//! // clipboard
//! pub enum ClipboardTool { Pbcopy, WlCopy, Xclip, Xsel, Clip }
//! pub struct ClipboardPolicy<'a> {
//!   pub tool_order: &'a [ClipboardTool],
//!   pub warn_on_failure: bool,
//! }
//! pub enum ClipboardOutcome { Copied(ClipboardTool), SkippedNoTool, SkippedFailure }
//! pub fn copy_best_effort(text: &str, policy: &ClipboardPolicy<'_>) -> ClipboardOutcome;
//! ```
//!
//! ## Compatibility rules
//! - `nils-common` returns structured results only; user-facing warning/error text stays in
//!   caller adapters.
//! - Exit-code mapping stays in caller crates.
//! - APIs stay domain-neutral and must not encode crate-specific UX policies.
//! - Quoting and ANSI differences are expressed via explicit mode/policy parameters.
//!
pub mod clipboard;
pub mod env;
pub mod fs;
pub mod git;
pub mod process;
pub mod shell;

pub fn greeting(name: &str) -> String {
    format!("Hello, {name}!")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn greeting_formats_name() {
        let result = greeting("Nils");
        assert_eq!(result, "Hello, Nils!");
    }
}
