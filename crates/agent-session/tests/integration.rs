#[path = "integration/board.rs"]
mod board;
#[path = "integration/cli.rs"]
mod cli;
#[path = "integration/coordination.rs"]
mod coordination;
#[path = "integration/coordination_server.rs"]
mod coordination_server;
#[path = "integration/diagnose.rs"]
mod diagnose;
#[path = "integration/metadata.rs"]
mod metadata;
#[path = "integration/retitle_v3.rs"]
mod retitle_v3;

/// The `main-agent` facade ships from the sibling `nils-main-agent` package, so
/// Cargo does not export `CARGO_BIN_EXE_main-agent` here. Resolve it as a
/// version-checked workspace sibling; a package-scoped run that did not build
/// it fails with the build command instead of silently testing nothing.
fn main_agent_bin() -> std::path::PathBuf {
    nils_test_support::bin::required_sibling("main-agent", "nils-main-agent")
}
