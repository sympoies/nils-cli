use std::io::Write;

pub fn require_allow_dangerous(caller: Option<&str>, stderr: &mut impl Write) -> bool {
    codex_core::exec::require_allow_dangerous(caller, stderr)
}

pub fn exec_dangerous(prompt: &str, caller: &str, stderr: &mut impl Write) -> i32 {
    codex_core::exec::exec_dangerous(prompt, caller, stderr)
}
