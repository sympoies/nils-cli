//! Runner admission for the library test executable.

use std::ffi::CStr;

const NEXTEST_MODE: &CStr = c"NEXTEST_EXECUTION_MODE";
const DIAGNOSTIC: &[u8] = b"error: agent-session library tests require nextest. Run: cargo nextest run -p nils-agent-session --lib\n";

// Run before libtest dispatches tests. Nextest sets its execution mode when
// discovering tests as well as when running them.
#[used]
// SAFETY: These sections contain C-ABI initializer function pointers.
#[cfg_attr(
    target_vendor = "apple",
    unsafe(link_section = "__DATA,__mod_init_func,mod_init_funcs")
)]
#[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".init_array"))]
static CHECK_TEST_RUNNER: extern "C" fn() = require_nextest;

extern "C" fn require_nextest() {
    // SAFETY: The C runtime has initialized the environment, and test threads
    // have yet to start. DIAGNOSTIC is a static, readable byte buffer.
    unsafe {
        let mode = libc::getenv(NEXTEST_MODE.as_ptr());
        if !mode.is_null() && CStr::from_ptr(mode) == c"process-per-test" {
            return;
        }
        libc::write(
            libc::STDERR_FILENO,
            DIAGNOSTIC.as_ptr().cast(),
            DIAGNOSTIC.len(),
        );
        libc::_exit(1);
    }
}

#[test]
fn unsupported_runner_exits_before_libtest_discovery() {
    use pretty_assertions::assert_eq;

    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env_remove("NEXTEST_EXECUTION_MODE")
        .arg("--list")
        .output()
        .expect("launch the library test executable");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, DIAGNOSTIC);
}
