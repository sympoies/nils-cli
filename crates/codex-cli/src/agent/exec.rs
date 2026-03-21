use std::io::Write;

pub use crate::runtime::ExecOptions;

pub fn require_allow_dangerous(caller: Option<&str>, stderr: &mut impl Write) -> bool {
    crate::runtime::require_allow_dangerous(caller, stderr)
}

pub fn exec_dangerous(prompt: &str, caller: &str, stderr: &mut impl Write) -> i32 {
    crate::runtime::exec_dangerous(prompt, caller, stderr)
}

pub fn exec_dangerous_with_options(
    prompt: &str,
    caller: &str,
    stderr: &mut impl Write,
    options: ExecOptions,
) -> i32 {
    crate::runtime::exec_dangerous_with_options(prompt, caller, stderr, options)
}
