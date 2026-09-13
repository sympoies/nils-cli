//! Development log tooling for the `nils-cli` workspace.
//!
//! A devlog is a directory of `YYYY-MM.md` month files plus a `README.md`
//! index, holding an append-only narrative of notable work. The conventions
//! are prose in that README; this crate is what makes them enforceable.

pub mod check;
pub mod entry;
pub mod index;
pub mod model;
pub mod search;

pub use model::{Devlog, DevlogError, EntryDate, Month};
