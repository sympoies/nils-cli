//! Shared GitHub webhook engine behind `repo hook` and `org hook`.
//!
//! The compare-and-swap write, the read-back, secret redaction, dry-run and
//! batch handling here are independent of where the hook lives: a
//! [`HookTarget`] only supplies the REST path prefix, name validation and the
//! identity scope, so another target kind can be added without reshaping them.
//!
//! GitHub REST references:
//! - <https://docs.github.com/en/rest/repos/webhooks>
//! - <https://docs.github.com/en/rest/orgs/webhooks>
//!
//! URL changes use `PATCH .../hooks/{id}/config` with only `url`: the
//! configuration endpoint updates just the keys it is given, whereas
//! `PATCH .../hooks/{id}` documents that an omitted secret is removed when a
//! webhook is updated. The active flag has no configuration-endpoint form, so
//! it is changed with `PATCH .../hooks/{id}` carrying only `active` (no
//! `config`), and every write is read back to confirm nothing else changed,
//! including that a configured secret is still set. The secret value is never
//! read into output, logged, or sent by `set-url` / `set-active`.

mod batch;
mod create;
mod engine;
mod target;

pub(super) use create::create;
pub(super) use engine::{Action, Kind, execute};
pub(super) use target::HookTarget;
