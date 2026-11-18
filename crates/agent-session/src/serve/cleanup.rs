//! Local retention runs on every serve role, independently of relay/board use.
use super::*;

const TMP_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub(super) async fn bind_listener_and_start_delete_tombstone_cleanup(
    context: &CliContext,
    bind: SocketAddr,
    tmux_bin: &Path,
) -> io::Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let cleanup_context = context.clone();
    std::thread::spawn(move || {
        let _ = cleanup_session_delete_tombstones(
            &cleanup_context,
            SESSION_DELETE_TOMBSTONE_CLEANUP_LIMIT,
        );
    });
    tokio::spawn(tmp_maintenance(context.clone(), tmux_bin.to_path_buf()));
    Ok(listener)
}

async fn tmp_maintenance(context: CliContext, tmux_bin: PathBuf) {
    let mut interval = tokio::time::interval(TMP_SWEEP_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        // The first tick is immediate; subsequent passes require no launches.
        interval.tick().await;
        let context = context.clone();
        let tmux_bin = tmux_bin.clone();
        let result = tokio::task::spawn_blocking(move || {
            use crate::runtime_environment::session_tmp as tmp;
            tmp::sweep(&context, &tmux_bin, tmp::grace_from_env(), None)
        })
        .await;
        match result {
            Ok(Ok(report)) => eprintln!(
                "session-tmp-sweep: removed={} retained={}",
                report.removed,
                report.retained.values().sum::<usize>()
            ),
            _ => eprintln!("warning: session-tmp-sweep-failed"),
        }
    }
}

#[cfg(test)]
mod tests;
