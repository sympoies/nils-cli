//! Claude has no app-server control handle: drain explicit queued intents
//! independently of usage/auth recovery and the auto-resume enabled flag.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use super::super::ServeState;

pub(super) async fn run(state: Arc<ServeState>) {
    loop {
        reconcile(&state).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn reconcile(state: &Arc<ServeState>) {
    let context = state.context.clone();
    let ids = tokio::task::spawn_blocking(move || {
        let Ok(entries) = std::fs::read_dir(context.state_dir.join("sessions")) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|id| crate::load_session_record(&context, &id).ok())
            .filter(|record| {
                record.agent == "claude" && crate::claude_account::has_queued_next(record)
            })
            .map(|record| record.id)
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let mut tasks = JoinSet::new();
    for id in ids {
        let state = state.clone();
        tasks.spawn(async move {
            let _switch = state.account_switches.lock(&id).await;
            let context = state.context.clone();
            let tmux = state.tmux_bin.clone();
            match tokio::task::spawn_blocking(move || {
                crate::session_account::claude_idle::apply_queued(&context, &id, &tmux)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!(
                    "warning: queued Claude account switch blocked: {}",
                    error.code()
                ),
                Err(_) => eprintln!("warning: queued Claude account worker failed"),
            }
        });
        if tasks.len() >= super::super::MAX_CONCURRENT_AUTO_RESUME_TICKS {
            let _ = tasks.join_next().await;
        }
    }
    while tasks.join_next().await.is_some() {}
}
