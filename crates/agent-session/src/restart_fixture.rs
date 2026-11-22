use super::*;
use std::os::unix::ffi::OsStrExt;

/// Own every pane launched by this fixture, including across failed restarts.
/// Cloning the executable path does not transfer teardown ownership.
pub(super) struct FakeTmux {
    bin: PathBuf,
    root: PathBuf,
    _clock: test_control::Fixture,
}

impl FakeTmux {
    pub(super) fn synchronous_observations(
        &self,
    ) -> (crate::tmux_probe_fixture::StatusGuard, fs::File) {
        let input = self.root.join(test_control::PANE_INPUT);
        let name = std::ffi::CString::new(input.as_os_str().as_bytes()).unwrap();
        // SAFETY: the name is a nul-terminated path in this fixture's temp dir.
        assert!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) } == 0);
        // Keep the writer open for the fixture's lifetime. Closing it also
        // releases the pane if the test stops before exercising provider exit.
        let input = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(input)
            .unwrap();
        let status = crate::tmux_probe_fixture::install_status(
            &self.bin,
            crate::tmux_probe_fixture::Status::RunningMarker(self.root.join("running")),
        );
        (status, input)
    }

    /// Wait for the pane to exit after the simulated provider receives Enter.
    pub(super) fn observe_provider_exit(
        &self,
        context: &CliContext,
        launch: &str,
    ) -> test_control::Hook {
        use std::os::fd::AsRawFd;

        let pid = fs::read_to_string(self.root.join("pane"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let pane = crate::open_linux_process_pidfd(pid).unwrap();
        let running = self.root.join("running");
        test_control::on(
            context,
            test_control::Phase::ExitRequested,
            launch,
            move |_, _| {
                assert!(
                    !running.exists(),
                    "the fixture provider did not handle the exit command"
                );
                let mut poll = libc::pollfd {
                    fd: pane.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // SAFETY: the descriptor belongs to this fixture's pinned pane.
                let ready = unsafe { libc::poll(&mut poll, 1, -1) };
                assert!(
                    ready == 1 && poll.revents & libc::POLLIN != 0,
                    "fixture pane exit was not observed"
                );
            },
        )
    }
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
/// `stall-status` marker stalls `has-session`. An `echo-paste` marker instead
/// models a composer accepting a fresh prompt and clearing it on Enter.
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
    if [ -p "$root/{pane_input}" ]; then
      # The pane reports its PID after setsid, then blocks without descendants.
      setsid sh -c '
        root=$1
        printf "%s\n" "$$" > "$root/pane"
        printf "%s\n" "$$" >> "$root/panes"
        : > "$root/running"
        printf "%s\t%s\t%s\n" "\$77" "%77" "$$"
        exec >/dev/null 2>&1
        IFS= read -r ignored < "$2"
      ' pane "$root" "$root/{pane_input}" </dev/null &
    else
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
    fi
    ;;
  display-message) printf '%s\t%s\t%s\n' '$77' '%77' "$(cat "$root/pane")" ;;
  has-session) [ -f "$root/stall-status" ] && {{ : > "$root/status-stalled"; sleep 20; }} ;;
  show-environment) [ -f "$root/stall" ] && sleep 20; grep "^$4=" "$root/env" || exit 1 ;;
  capture-pane) cat "$root/screen" ;;
  load-buffer) cat "$4" >> "$root/pasted-input" ;;
  paste-buffer) [ -f "$root/echo-paste" ] && printf '%s\n' '────────────────────' "❯$(printf '\302\240')$(tail -n 1 "$root/pasted-input")" '────────────────────' '  status' > "$root/screen" ;;
  send-keys) case " $* " in *" Enter "*)
    if [ -f "$root/clear-requested" ]; then rm -f "$root/clear-requested"; exit 0; fi
    if [ -f "$root/exit-dialog" ]; then cp "$root/exit-dialog" "$root/screen"; exit 0; fi
    if [ -f "$root/echo-paste" ]; then
      printf '%s\n' '────────────────────' "❯$(printf '\302\240')" '────────────────────' '  status' > "$root/screen"
    else [ -f "$root/hold-exit" ] || stop_pane; fi ;;
    *" Escape "*)
      if [ -f "$root/exit-dialog" ] && cmp -s "$root/screen" "$root/exit-dialog"; then
        cp "$root/after-escape" "$root/screen"
        if [ -f "$root/escape-exits" ]; then stop_pane; fi
      fi ;;
  esac ;;
  if-shell|kill-session) stop_pane ;;
esac
exit 0
"#,
        root = shell_words::quote(&root.to_string_lossy()),
        owner_pid = std::process::id(),
        pane_input = test_control::PANE_INPUT,
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
        _clock: test_control::install(root),
    }
}
