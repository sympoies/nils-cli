//! Stateful tmux fixtures for create tests; resume-only callers retain their existing probe.
use super::{Path, PathBuf, executable, live_pane_stub_prelude};

pub(super) fn resume_tmux(dir: &Path, log: &Path) -> PathBuf {
    fixture_tmux(
        dir,
        log,
        r#"printf '%s\n' "can't find session: fixture" >&2; exit 1"#,
    )
}

pub(super) fn live_resume_tmux(dir: &Path, log: &Path) -> PathBuf {
    fixture_tmux(
        dir,
        log,
        r#"if [ -s "$NILS_TEST_PANE_PARENT/.pane-identity" ]; then set -- $(cat "$NILS_TEST_PANE_PARENT/.pane-identity"); kill -0 "$1" 2>/dev/null && exit 0; fi; printf '%s\n' "can't find session: fixture" >&2; exit 1"#,
    )
}

fn fixture_tmux(dir: &Path, log: &Path, probe: &str) -> PathBuf {
    let pane_stub = live_pane_stub_prelude(dir);
    executable(
        &dir.join("tmux"),
        &format!(
            "#!/usr/bin/env sh\n{pane_stub}\nprintf '%s\\n' \"$*\" >> {}\ncase \"$1\" in\n  has-session) {probe} ;;\n  new-session) heartbeat=''; slot=0; for arg in \"$@\"; do if [ \"$slot\" = 2 ]; then incarnation=\"$arg\"; break; elif [ \"$slot\" = 1 ]; then slot=2; else case \"$arg\" in */coordination/heartbeat) heartbeat=\"$arg\"; slot=1 ;; esac; fi; done; if [ -n \"$heartbeat\" ] && [ -n \"$incarnation\" ]; then mkdir -p \"$(dirname \"$heartbeat\")\"; printf '%s:%s\\n' \"$incarnation\" \"$(date +%s)\" > \"$heartbeat\"; chmod 600 \"$heartbeat\"; fi; start_live_pane || exit 1; printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$NILS_TEST_PANE_PID\"; exit 0 ;;\n  display-message) set -- $(cat \"$NILS_TEST_PANE_PARENT/.pane-identity\"); printf '%s\\t%s\\t%s\\n' '$77' '%77' \"$1\"; exit 0 ;;\n  *) exit 0 ;;\nesac\n",
            shell_words::quote(&log.to_string_lossy())
        ),
    )
}
