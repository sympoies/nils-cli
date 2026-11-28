//! Tmux inventory for daemon observers. Lifecycle actions still probe live.
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use super::ServeState;
use crate::{
    CliContext, SessionRecord,
    auth_incident::{self, PanePoll, PaneSample, TerminalObservation},
    tmux_inventory,
};

pub(super) fn records(context: &CliContext) -> impl Iterator<Item = SessionRecord> + '_ {
    std::fs::read_dir(context.state_dir.join("sessions"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|id| crate::load_session_record(context, &id).ok())
}

pub(super) fn discover_codex_controls(context: &CliContext, tmux: &Path) -> Vec<SessionRecord> {
    let mut candidates = records(context)
        .filter(crate::codex_app_server::runtime_is_supported)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return candidates;
    }
    let Some(snapshots) = crate::tmux_session_snapshots(tmux) else {
        return Vec::new();
    };
    candidates.retain(|record| snapshots.contains_key(&record.tmux_session));
    candidates
}

pub(super) async fn auth_incident_loop(state: Arc<ServeState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut sampled = HashMap::<String, PaneSample>::new();
    loop {
        interval.tick().await;
        let context = state.context.clone();
        let discovery_tmux = state.tmux_bin.clone();
        let Ok((records, snapshot)) = tokio::task::spawn_blocking(move || {
            let records = records(&context)
                .filter(|record| {
                    record.mode == "interactive"
                        && matches!(record.agent.as_str(), "claude" | "codex")
                        && record.runtime.is_some()
                })
                .collect::<Vec<_>>();
            let snapshot = (!records.is_empty())
                .then(|| tmux_inventory::bounded_snapshot(&discovery_tmux))
                .flatten();
            (records, snapshot)
        })
        .await
        else {
            sampled.clear();
            continue;
        };
        // Bound concurrent sampling and mailbox requests. Retain cache entries
        // only for live, still-enrolled records returned by this tick.
        let mut tasks = JoinSet::new();
        let mut next = HashMap::new();
        for record in records {
            if tasks.len() >= 4 {
                collect_sample(&mut tasks, &mut next).await;
            }
            let mut terminal = match &snapshot {
                Some(snapshot) if snapshot.contains_key(&record.tmux_session) => {
                    TerminalObservation::Live(PanePoll {
                        activity: snapshot
                            .last_activity(&record.tmux_session)
                            .map(str::to_owned),
                        sample: sampled.remove(&record.id),
                    })
                }
                Some(_) => TerminalObservation::Absent,
                None => TerminalObservation::Unavailable,
            };
            let context = state.context.clone();
            let tmux = state.tmux_bin.clone();
            let machine = state.machine.clone();
            tasks.spawn_blocking(move || {
                if let Err(error) =
                    auth_incident::tick(&context, &record, &tmux, &machine, &mut terminal)
                {
                    eprintln!(
                        "warning: provider authentication observation degraded: {}",
                        error.code()
                    );
                    return None;
                }
                if let TerminalObservation::Live(pane) = terminal {
                    pane.sample.map(|sample| (record.id, sample))
                } else {
                    None
                }
            });
        }
        while !tasks.is_empty() {
            collect_sample(&mut tasks, &mut next).await;
        }
        sampled = next;
    }
}

async fn collect_sample(
    tasks: &mut JoinSet<Option<(String, PaneSample)>>,
    sampled: &mut HashMap<String, PaneSample>,
) {
    if let Some(Ok(Some((id, sample)))) = tasks.join_next().await {
        sampled.insert(id, sample);
    }
}
