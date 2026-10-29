//! `secrets` — pull, add, and set dotenv entries in a SOPS store.
//!
//! `pull` redirects decrypted values to a mode-600 file. `set` updates one
//! assignment in memory and atomically installs validated ciphertext, then
//! explicitly hands off signed commit/PR delivery. Values never enter stdout,
//! diagnostics, JSON metadata, or delivery text. See the crate README.

pub mod cli;
pub mod completion;

pub mod runtime;
mod setter;
pub mod store;

pub fn run() -> i32 {
    cli::run()
}
