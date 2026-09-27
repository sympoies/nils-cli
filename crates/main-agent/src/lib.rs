//! `main-agent`: the typed, authenticated facade for durable Main Agent
//! orchestration runs and their interactive managed workers.
//!
//! The session engine, orchestration registry, and group lifecycle live in
//! `nils-agent-session`; this crate owns the facade's command surface and its
//! controller and worker workflows. It reaches the engine only through
//! `agent_session::internal`, an unstable surface shared by these two
//! workspace crates.

mod main_agent;

pub use main_agent::{run, run_with_args};
