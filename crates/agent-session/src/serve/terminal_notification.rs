//! Mailbox notices use the guarded current-envelope route.
use super::*;

mod attested;
mod mailbox_dispatch;
mod native_notice;
pub(super) use mailbox_dispatch::dispatch_coordination_notification;
pub(crate) use native_notice::begin_mailbox_notice;
