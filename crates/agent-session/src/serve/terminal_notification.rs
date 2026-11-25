//! Mailbox notices use the guarded current-envelope route.
use super::*;

mod attested;
mod mailbox_dispatch;
mod native_notice;
pub(super) use mailbox_dispatch::dispatch_coordination_notification;
pub(crate) use native_notice::begin_mailbox_notice;

pub(super) fn unresolved_attempts(
    context: &CliContext,
) -> Result<Vec<(crate::coordination::NotificationCandidate, bool)>, CliError> {
    let _ = crate::send::claude_notice::store::reconcile(context);
    crate::coordination::unresolved_notification_attempts(context)
}
#[cfg(test)]
pub(in crate::serve) async fn dispatch_native_notice_for_test(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
) {
    native_notice::dispatch(state, record, candidate).await;
}
