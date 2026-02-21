use std::io::Write;

pub fn require_allow_dangerous(caller: Option<&str>, stderr: &mut impl Write) -> bool {
    crate::runtime::require_allow_dangerous(caller, stderr)
}

pub fn exec_dangerous(prompt: &str, caller: &str, stderr: &mut impl Write) -> i32 {
    crate::runtime::exec_dangerous(prompt, caller, stderr)
}
