//! Mailbox notices use the guarded current-envelope route.
use super::*;

mod attested;
mod mailbox_dispatch;
mod native_notice;
pub(super) use mailbox_dispatch::dispatch_coordination_notification;
pub(crate) use native_notice::begin_mailbox_notice;
#[cfg(test)]
pub(in crate::serve) async fn dispatch_native_notice_for_test(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
) {
    native_notice::dispatch(state, record, candidate).await;
}
