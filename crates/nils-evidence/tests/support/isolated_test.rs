//! Run in-process evidence scenarios in a private child environment.

use nils_test_support::fixture::Fixture;

const CHILD: &str = "NILS_EVIDENCE_ISOLATED_TEST";

pub fn run(name: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let fixture = Fixture::new();
    let status = fixture
        .command(std::env::current_exe().expect("test executable"))
        .env(CHILD, "1")
        .args(["--exact", name, "--nocapture"])
        .status()
        .expect("run isolated test");
    assert!(status.success(), "isolated test {name} failed: {status}");
    true
}
