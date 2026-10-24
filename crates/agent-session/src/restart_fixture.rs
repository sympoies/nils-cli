use super::*;

/// Own every pane launched by this fixture, including across failed restarts.
/// Cloning the executable path does not transfer teardown ownership.
pub(super) struct FakeTmux {
    bin: PathBuf,
    root: PathBuf,
}

impl std::ops::Deref for FakeTmux {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.bin
    }
}

impl Drop for FakeTmux {
    fn drop(&mut self) {
        if let Ok(panes) = fs::read_to_string(self.root.join("panes")) {
            for group in panes
                .lines()
                .filter_map(|pid| pid.parse::<libc::pid_t>().ok())
            {
                if group > 1 {
                    // Each fake pane starts in a dedicated process group.
                    unsafe {
                        libc::kill(-group, libc::SIGKILL);
                    }
                }
            }
        }
    }
}

/// A tmux fake whose session exists only after `new-session`. Like the real
/// held launch, the new incarnation's heartbeat starts once the broker gate
/// opens, so the prior incarnation's beat is the one on disk while a resume
/// provisions coordination. An Enter key stands in for the provider exiting on
/// its exit command unless `hold-exit` exists, a `fail-launch` marker makes the
/// next launch fail, a `stall` marker stalls pane identity observations and a
/// `stall-status` marker stalls `has-session`.
pub(super) fn fake_tmux(root: &Path) -> FakeTmux {
    let script = format!(
        r#"#!/bin/sh
root={root}
printf '%s\n' "$*" >> "$root/tmux.log"
stop_pane() {{ kill -- "-$(cat "$root/pane")" 2>/dev/null; rm -f "$root/running"; }}
if [ -f "$root/exit-on-probe" ] && [ "$1" = display-message ]; then
  stop_pane
  rm -f "$root/exit-on-probe"
fi
if [ "$1" != new-session ] && [ ! -f "$root/running" ]; then
  printf '%s\n' "can't find session: fixture" >&2
  exit 1
fi
case "$1" in
  new-session)
    [ -f "$root/fail-launch" ] && exit 1
    heartbeat=''; slot=0; env=''
    : > "$root/env"
    for arg in "$@"; do
      if [ "$env" = 1 ]; then printf '%s\n' "$arg" >> "$root/env"; env=''; continue; fi
      [ "$arg" = -e ] && env=1
      if [ "$slot" = 2 ]; then incarnation="$arg"; slot=3
      elif [ "$slot" = 1 ]; then slot=2
      elif [ "$slot" = 0 ]; then case "$arg" in */coordination/heartbeat) heartbeat="$arg"; slot=1 ;; esac
      fi
    done
    if [ -n "$heartbeat" ]; then
      gate="$(dirname "$heartbeat")/broker-provisioned"
      (i=0; while [ ! -f "$gate" ] && [ "$i" -lt 1000 ]; do i=$((i + 1)); sleep 0.01; done
       printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"; chmod 600 "$heartbeat") >/dev/null 2>&1 &
    fi
    setsid sh -c 'while [ -d "$1" ] && kill -0 "$2" 2>/dev/null; do sleep 0.05; done' pane "$root" {owner_pid} >/dev/null 2>&1 </dev/null &
    printf '%s\n' "$!" > "$root/pane"
    printf '%s\n' "$!" >> "$root/panes"
    : > "$root/running"
    printf '%s\t%s\t%s\n' '$77' '%77' "$(cat "$root/pane")"
    ;;
  display-message) printf '%s\t%s\t%s\n' '$77' '%77' "$(cat "$root/pane")" ;;
  has-session) [ -f "$root/stall-status" ] && {{ : > "$root/status-stalled"; sleep 20; }} ;;
  show-environment) [ -f "$root/stall" ] && sleep 20; grep "^$4=" "$root/env" || exit 1 ;;
  capture-pane) cat "$root/screen" ;;
  load-buffer) cat "$4" >> "$root/pasted-input" ;;
  send-keys) case " $* " in *" Enter "*) [ -f "$root/hold-exit" ] || stop_pane ;; esac ;;
  if-shell|kill-session) stop_pane ;;
esac
exit 0
"#,
        root = shell_words::quote(&root.to_string_lossy()),
        owner_pid = std::process::id(),
    );
    // An idle Claude Code composer with nothing typed.
    fs::write(
        root.join("screen"),
        "────────────────────\n❯\u{a0}\n────────────────────\n  status\n",
    )
    .unwrap();
    FakeTmux {
        bin: executable(&root.join("tmux"), &script),
        root: root.to_path_buf(),
    }
}
