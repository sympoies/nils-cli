#![allow(dead_code, unused_imports)]
#[path = "../src/agent/exec.rs"]
mod exec;

use std::sync::{Mutex, OnceLock};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
}

struct EnvGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var_os(key);
        // SAFETY: tests mutate process env with a global lock.
        unsafe { std::env::set_var(key, value) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.old.take() {
            // SAFETY: tests mutate process env with a global lock.
            unsafe { std::env::set_var(self.key, value) };
        } else {
            // SAFETY: tests mutate process env with a global lock.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

#[test]
fn exec_dangerous_missing_prompt_exits_1() {
    let mut stderr: Vec<u8> = Vec::new();
    let code = exec::exec_dangerous("", "caller", &mut stderr);
    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("_gemini_exec_dangerous: missing prompt"));
}

#[test]
fn require_allow_dangerous_without_caller_uses_gemini_prefix() {
    let _lock = env_lock();
    let _danger = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "false");

    let mut stderr: Vec<u8> = Vec::new();
    let allowed = exec::require_allow_dangerous(None, &mut stderr);
    assert!(!allowed);
    assert!(
        String::from_utf8_lossy(&stderr)
            .contains("gemini: disabled (set GEMINI_ALLOW_DANGEROUS_ENABLED=true)")
    );
}

#[test]
fn require_allow_dangerous_warns_on_invalid_value() {
    let _lock = env_lock();
    let _danger = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "wat");

    let mut stderr: Vec<u8> = Vec::new();
    let allowed = exec::require_allow_dangerous(Some("caller"), &mut stderr);
    assert!(!allowed);
    let err = String::from_utf8_lossy(&stderr);
    assert!(err.contains("warning: GEMINI_ALLOW_DANGEROUS_ENABLED must be true|false"));
    assert!(err.contains("caller: disabled"));
}
