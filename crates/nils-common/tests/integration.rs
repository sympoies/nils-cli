// Consolidated integration test target.
// Each former `tests/*.rs` is declared as a submodule here so the crate
// links one integration test binary instead of many. This keeps the
// dev-loop link phase O(crates) instead of O(test-files).

#[path = "integration/cli_contract.rs"]
mod cli_contract;
#[path = "integration/forge_identity.rs"]
mod forge_identity;
#[path = "integration/markdown_table_canonicalization.rs"]
mod markdown_table_canonicalization;
#[path = "integration/provider_runtime_contract.rs"]
mod provider_runtime_contract;

#[cfg(unix)]
#[path = "integration/forge_identity_remote.rs"]
mod forge_identity_remote;

#[cfg(unix)]
#[path = "integration/forge_identity_unasserted.rs"]
mod forge_identity_unasserted;

#[cfg(unix)]
#[path = "integration/forge_identity_https.rs"]
mod forge_identity_https;

#[cfg(unix)]
#[path = "integration/forge_identity_children.rs"]
mod forge_identity_children;

#[cfg(unix)]
#[path = "integration/forge_identity_network.rs"]
mod forge_identity_network;

#[cfg(unix)]
#[path = "integration/forge_identity_verification_cache.rs"]
mod forge_identity_verification_cache;
