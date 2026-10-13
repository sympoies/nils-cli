//! Retain the notice cursor across daemon outbox wakes.
use std::sync::Arc;
use std::time::Duration;

use super::ServeState;

pub(super) async fn run(state: Arc<ServeState>) {
    let Some(config) = state.federation.clone() else {
        return;
    };
    let mut notices = crate::coordination::remote::NoticeState::default();
    loop {
        let context = state.context.clone();
        let config = config.clone();
        let mut notice_state = std::mem::take(&mut notices);
        let deadline = match tokio::task::spawn_blocking(move || {
            let result = crate::coordination::remote::drain_with_notice_state(
                &context,
                &config,
                &mut notice_state,
            );
            (notice_state, result)
        })
        .await
        {
            Ok((notice_state, result)) => {
                notices = notice_state;
                result.unwrap_or_else(|_| Some(crate::coordination::now_epoch().saturating_add(5)))
            }
            _ => Some(crate::coordination::now_epoch().saturating_add(5)),
        };
        // Notify retains an enqueue permit even if submission races with drain.
        if let Some(deadline) = deadline {
            let seconds = deadline
                .saturating_sub(crate::coordination::now_epoch())
                .max(0) as u64;
            tokio::select! {
                _ = state.remote_outbox_wake.notified() => {},
                _ = tokio::time::sleep(Duration::from_secs(seconds)) => {},
            }
        } else {
            state.remote_outbox_wake.notified().await;
        }
    }
}
