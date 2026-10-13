use pretty_assertions::assert_eq;
use std::fs;
use std::path::{Path, PathBuf};

use super::{prewarm_fake_tmux, write_executable};

pub(crate) fn fake_tmux(tmp: &Path) -> (PathBuf, PathBuf) {
    let bin = tmp.join("tmux");
    let log = tmp.join("tmux.log");
    let pane_parent_path = tmp.to_string_lossy();
    let pane_parent = shell_words::quote(&pane_parent_path);
    let pane_start_time = if cfg!(target_os = "linux") {
        r#"stat = open("/proc/self/stat", encoding="utf-8").read()
start_time = stat[stat.rfind(") ") + 2:].split()[19]"#
    } else {
        r#"start_time = "0""#
    };
    write_executable(
        &bin,
        &r##"#!/usr/bin/env sh
: "${AGENT_SESSION_FAKE_TMUX_LOG:?}"
# Serialize the diagnostic record only; fake tmux operations remain concurrent.
# A bounded probe may be SIGKILLed while logging. Kernel-owned flock releases
# on process exit; a noclobber-created file can strand every later probe.
python3 -c '
import fcntl
import os
import sys
import time

log = os.environ["AGENT_SESSION_FAKE_TMUX_LOG"]
with open(log + ".call-lock", "ab") as lock:
    deadline = time.monotonic() + 10
    while True:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                print("fake tmux call log lock timed out", file=sys.stderr)
                sys.exit(1)
            time.sleep(min(0.01, remaining))
    record = b"".join(os.fsencode(arg) + b"\0" for arg in sys.argv[1:]) + b"\x1e"
    with open(log, "ab", buffering=0) as output:
        if output.write(record) != len(record):
            raise OSError("incomplete fake tmux diagnostic record")
' "$@" || exit 1

NILS_TEST_PANE_PARENT=__NILS_TEST_PANE_PARENT__
start_live_pane() {
  if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'fake tmux live-pane fixture requires python3' >&2
    return 127
  fi
  ready="$NILS_TEST_PANE_PARENT/.pane-ready-$$"
  rm -f "$ready"
  NILS_TEST_PANE_PARENT="$NILS_TEST_PANE_PARENT" NILS_TEST_PANE_READY="$ready" NILS_TEST_PANE_LIFETIME_MS="${NILS_TEST_PANE_LIFETIME_MS:-30000}" python3 -c '
import os
import time
os.setsid()
ready = os.environ["NILS_TEST_PANE_READY"]
temporary = ready + ".tmp." + str(os.getpid())
__NILS_TEST_PANE_START_TIME__
with open(temporary, "x", encoding="utf-8") as handle:
    handle.write(str(os.getpid()) + " " + start_time + "\n")
os.replace(temporary, ready)
deadline = time.monotonic() + int(os.environ["NILS_TEST_PANE_LIFETIME_MS"]) / 1000
parent = os.environ["NILS_TEST_PANE_PARENT"]
while os.path.isdir(parent) and time.monotonic() < deadline:
    time.sleep(0.02)
' >/dev/null 2>&1 &
  launcher=$!
  attempts=0
  while [ ! -s "$ready" ]; do
    attempts=$((attempts + 1))
    if [ "$attempts" -ge 200 ] || ! kill -0 "$launcher" 2>/dev/null; then
      kill "$launcher" 2>/dev/null || true
      wait "$launcher" 2>/dev/null || true
      rm -f "$ready"
      return 1
    fi
    sleep 0.01
  done
  set -- $(cat "$ready")
  NILS_TEST_PANE_PID=$1
  NILS_TEST_PANE_START_TIME=$2
  rm -f "$ready"
  [ -n "$NILS_TEST_PANE_PID" ] && [ -n "$NILS_TEST_PANE_START_TIME" ] &&
    kill -0 "$NILS_TEST_PANE_PID" 2>/dev/null
}

pane_process_start_time() {
  pane_pid="$1"
  if [ -r "/proc/$pane_pid/stat" ]; then
    sed 's/^.*) //' "/proc/$pane_pid/stat" 2>/dev/null | awk '{ print $20 }'
    return
  fi
  LC_ALL=C ps -o lstart= -p "$pane_pid" 2>/dev/null | cksum | awk '{ print $1 ":" $2 }'
}

terminate_exact_pane() {
  pane_target="$1"
  pane_mapping="$(awk -F '\t' -v target="$pane_target" '
    $1 == target || $2 == target {
      count += 1
      value = $4 "\t" $5
    }
    END {
      if (count == 1) print value
    }
  ' "$AGENT_SESSION_FAKE_TMUX_LOG.panes" 2>/dev/null)"
  [ -n "$pane_mapping" ] || return 0
  old_ifs="$IFS"
  IFS='	'
  set -- $pane_mapping
  IFS="$old_ifs"
  pane_pid="$1"
  expected_start_time="$2"
  [ -n "$pane_pid" ] && [ -n "$expected_start_time" ] || return 0
  observed_start_time="$(pane_process_start_time "$pane_pid")"
  [ "$observed_start_time" = "$expected_start_time" ] || return 0
  observed_pgid="$(LC_ALL=C ps -o pgid= -p "$pane_pid" 2>/dev/null | tr -d '[:space:]')"
  [ "$observed_pgid" = "$pane_pid" ] || return 0
  kill -TERM "-$pane_pid" 2>/dev/null || true
}

operation="$1"
if [ "$operation" = "if-shell" ]; then
  operation="kill-session"
fi
if [ "${AGENT_SESSION_FAKE_TMUX_FAIL:-}" = "$operation" ]; then
  fail_once_dir="${AGENT_SESSION_FAKE_TMUX_FAIL_ONCE_DIR:-}"
  if [ -z "$fail_once_dir" ] || mkdir "$fail_once_dir" 2>/dev/null; then
    echo "fake tmux failed at $operation" >&2
    exit 42
  fi
fi

if [ "${AGENT_SESSION_FAKE_TMUX_ABSENT:-0}" = "1" ] && { [ "$1" = "display-message" ] || [ "$1" = "has-session" ]; }; then
  printf "%s\n" "can't find session: ${target:-unknown}" >&2
  exit 1
fi

if [ "${AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH:-0}" = "1" ] && { [ "$1" = "display-message" ] || [ "$1" = "has-session" ]; } && [ ! -f "$AGENT_SESSION_FAKE_TMUX_LOG.launched" ]; then
  printf "%s\n" "can't find session: ${target:-unknown}" >&2
  exit 1
fi

if [ "${AGENT_SESSION_FAKE_TMUX_BLANK_DISPLAY:-0}" = "1" ] && [ "$1" = "display-message" ]; then
  exit 0
fi

target=""
previous=""
for arg in "$@"; do
  if [ "$previous" = "-t" ]; then
    target="$arg"
    break
  fi
  previous="$arg"
done

if [ "$1" = "kill-session" ]; then
  printf '%s\n' "$target" >> "$AGENT_SESSION_FAKE_TMUX_LOG.killed"
  if [ -n "${AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID:-}" ] && [ "${AGENT_SESSION_FAKE_TMUX_KEEP_PROCESS_GROUP:-0}" != "1" ]; then
    kill -TERM "-${AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID}" 2>/dev/null || true
  elif [ "${AGENT_SESSION_FAKE_TMUX_KEEP_PROCESS_GROUP:-0}" != "1" ]; then
    terminate_exact_pane "$target"
  fi
  exit 0
fi

if [ "$1" = "if-shell" ]; then
  case "$6" in
    'run-shell '*)
      # Model only the exact guarded command and fixture-owned live identity.
      # Command text is compared as data; it is never evaluated by a shell.
      kill_target="$(python3 -c '
import os
import re
import signal
import sys

def require(condition):
    if not condition:
        raise ValueError("guarded fixture identity mismatch")

try:
    args = sys.argv[1:]
    require(len(args) == 7 and args[:3] == ["if-shell", "-F", "-t"])
    pane = os.environ.get("AGENT_SESSION_FAKE_TMUX_PANE_ID", "%77")
    session = os.environ.get("AGENT_SESSION_FAKE_TMUX_SESSION_ID", "$77")
    pid = int(os.environ.get("AGENT_SESSION_FAKE_TMUX_PANE_PID", "0"))
    group = int(os.environ.get("AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID", "0"))
    rows = []
    try:
        with open(os.environ["AGENT_SESSION_FAKE_TMUX_LOG"] + ".panes") as source:
            rows = [line.rstrip("\n").split("\t") for line in source]
    except FileNotFoundError:
        pass
    matched = [row for row in rows if len(row) == 5 and row[2] == args[3]]
    if matched:
        _, session, pane, pid, _ = matched[-1]
        pid = int(pid)
        group = os.getpgid(pid)
    require(re.fullmatch(r"\$[0-9]+", session) and re.fullmatch(r"%[0-9]+", pane))
    require(pid > 1 and group > 1 and group != os.getpgrp())
    require(os.getpgid(pid) == group and args[3] == pane)
    pid_clause = "#{==:#{pane_pid},%s}" % pid
    pane_clause = "#{&&:#{==:#{pane_id},%s},%s}" % (pane, pid_clause)
    identity_clause = "#{&&:#{==:#{session_id},%s},%s}" % (session, pane_clause)
    condition = "#{&&:#{==:#{pane_dead},0},%s}" % identity_clause
    require(args[4] == condition)
    require(args[5] == f"run-shell \"/bin/kill -KILL -- -{group}\"; kill-session -t {session}")
    require(args[6] == "display-message -p agent-session-runtime-identity-changed")
    if os.environ.get("AGENT_SESSION_FAKE_TMUX_KEEP_PROCESS_GROUP", "0") != "1":
        os.killpg(group, signal.SIGKILL)
    print(session)
except (AssertionError, KeyError, OSError, ValueError):
    sys.exit(42)
' "$@")" || exit 42
      printf '%s\n' "$kill_target" >> "$AGENT_SESSION_FAKE_TMUX_LOG.killed"
      exit 0
      ;;
  esac
  kill_target=""
  for arg in "$@"; do
    case "$arg" in
      'kill-session -t '*) kill_target="${arg#kill-session -t }" ;;
    esac
  done
  [ -n "$kill_target" ] || exit 42
  printf '%s\n' "$kill_target" >> "$AGENT_SESSION_FAKE_TMUX_LOG.killed"
  if [ -n "${AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID:-}" ] && [ "${AGENT_SESSION_FAKE_TMUX_KEEP_PROCESS_GROUP:-0}" != "1" ]; then
    kill -TERM "-${AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID}" 2>/dev/null || true
  elif [ "${AGENT_SESSION_FAKE_TMUX_KEEP_PROCESS_GROUP:-0}" != "1" ]; then
    terminate_exact_pane "$kill_target"
  fi
  exit 0
fi

target_was_killed() {
  normalized_target="${target#=}"
  normalized_target="${normalized_target%%:*}"
  mapped_session="$(awk -F '\t' -v target="$normalized_target" '$1 == target { value=$2 } END { print value }' "$AGENT_SESSION_FAKE_TMUX_LOG.identities" 2>/dev/null)"
  [ -f "$AGENT_SESSION_FAKE_TMUX_LOG.killed" ] || return 1
  while IFS= read -r killed_target; do
    normalized_killed="${killed_target#=}"
    normalized_killed="${normalized_killed%%:*}"
    if [ "$normalized_killed" = "$normalized_target" ] || { [ -n "$mapped_session" ] && [ "$normalized_killed" = "$mapped_session" ]; }; then
      return 0
    fi
  done < "$AGENT_SESSION_FAKE_TMUX_LOG.killed"
  return 1
}

if [ "${AGENT_SESSION_FAKE_TMUX_ABSENT_AFTER_KILL:-0}" = "1" ] && { [ "$1" = "display-message" ] || [ "$1" = "has-session" ]; } && target_was_killed; then
  printf "%s\n" "can't find session: $target" >&2
  exit 1
fi

if [ "$1" = "has-session" ]; then
  if [ "${AGENT_SESSION_FAKE_TMUX_HAS_SESSION:-1}" = "0" ] || target_was_killed; then
    printf "%s\n" "can't find session: $target" >&2
    exit 1
  fi
  exit 0
fi

if [ "$1" = "display-message" ]; then
  last_arg=""
  for arg in "$@"; do
    last_arg="$arg"
  done
  case "$last_arg" in
    *'#{pane_pid}')
      tmux_name="${target#=}"
      tmux_name="${tmux_name%%:*}"
      mapped_identity="$(awk -F '\t' -v target="$tmux_name" '$1 == target { value=$2 "\t" $3 "\t" $4 } END { print value }' "$AGENT_SESSION_FAKE_TMUX_LOG.identities" 2>/dev/null)"
      if [ -n "$mapped_identity" ]; then
        if [ "${AGENT_SESSION_FAKE_TMUX_DRIFT_LAUNCH_IDENTITY:-0}" = "1" ]; then
          old_ifs="$IFS"
          IFS='	'
          set -- $mapped_identity
          IFS="$old_ifs"
          printf '%s\t%s\t%s\n' "$1" "$2" "$(($3 + 1))"
          exit 0
        fi
        printf '%s\n' "$mapped_identity"
        exit 0
      fi
      [ -n "${AGENT_SESSION_FAKE_TMUX_PANE_PID:-}" ] || exit 0
      session_identity="${AGENT_SESSION_FAKE_TMUX_SESSION_ID:-}"
      [ -n "$session_identity" ] || session_identity='$77'
      pane_identity="${AGENT_SESSION_FAKE_TMUX_PANE_ID:-}"
      [ -n "$pane_identity" ] || pane_identity='%77'
      case "$tmux_name" in
        ''|'$'*) ;;
        *) printf '%s\t%s\t%s\t%s\n' "$tmux_name" "$session_identity" "$pane_identity" "$AGENT_SESSION_FAKE_TMUX_PANE_PID" >> "$AGENT_SESSION_FAKE_TMUX_LOG.identities" ;;
      esac
      printf '%s\t%s\t%s\n' "$session_identity" "$pane_identity" "$AGENT_SESSION_FAKE_TMUX_PANE_PID"
      exit 0
      ;;
  esac
fi

if [ "$1" = "show-environment" ]; then
  tmux_name="${3#=}"
  tmux_name="${tmux_name%%:*}"
  mapped_value="$(awk -F '\t' -v target="$tmux_name" -v key="$4" '$1 == target && $2 == key { value=$3 } END { print value }' "$AGENT_SESSION_FAKE_TMUX_LOG.environments" 2>/dev/null)"
  if [ -n "$mapped_value" ]; then
    printf '%s=%s\n' "$4" "$mapped_value"
    exit 0
  fi
  case "$4" in
    AGENT_SESSION_ID) printf 'AGENT_SESSION_ID=%s\n' "$AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID" ;;
    AGENT_SESSION_STATE_DIR) printf 'AGENT_SESSION_STATE_DIR=%s\n' "$AGENT_SESSION_FAKE_TMUX_STATE_DIR" ;;
    AGENT_SESSION_RUNTIME_ID)
      runtime_id="${AGENT_SESSION_FAKE_TMUX_RUNTIME_ID:-}"
      if [ -z "$runtime_id" ] && [ -n "${AGENT_SESSION_FAKE_TMUX_RUNTIME_RECORD:-}" ]; then
        runtime_id="$(sed -n 's/.*"launch_id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$AGENT_SESSION_FAKE_TMUX_RUNTIME_RECORD" | head -n 1)"
      fi
      printf 'AGENT_SESSION_RUNTIME_ID=%s\n' "$runtime_id" ;;
    *) exit 1 ;;
  esac
  exit 0
fi

if [ "$1" = "capture-pane" ]; then
  if [ -n "${AGENT_SESSION_FAKE_TMUX_CAPTURE_SLEEP:-}" ]; then
    sleep "$AGENT_SESSION_FAKE_TMUX_CAPTURE_SLEEP"
  fi
  if [ "${AGENT_SESSION_FAKE_TMUX_CAPTURE+x}" = "x" ]; then
    printf '%s' "$AGENT_SESSION_FAKE_TMUX_CAPTURE"
    exit 0
  fi
  printf 'pane one\npane two\n'
  exit 0
fi

if [ "$1" = "list-windows" ]; then
  if [ "${AGENT_SESSION_FAKE_TMUX_LIST_WINDOWS+x}" = "x" ]; then
    printf '%s\n' "$AGENT_SESSION_FAKE_TMUX_LIST_WINDOWS"
    exit 0
  fi
  exit 1
fi

if [ "$1" = "display-message" ] && [ "${AGENT_SESSION_FAKE_TMUX_WINDOW_ACTIVITY+x}" = "x" ]; then
  printf '%s\n' "$AGENT_SESSION_FAKE_TMUX_WINDOW_ACTIVITY"
  exit 0
fi

if [ "$1" = "new-session" ] && [ "${AGENT_SESSION_FAKE_CODEX_SESSION_FILE+x}" = "x" ]; then
  cwd="${AGENT_SESSION_FAKE_CODEX_CWD:-}"
  if [ -z "$cwd" ]; then
    previous=""
    for arg in "$@"; do
      if [ "$previous" = "-c" ]; then
        cwd="$arg"
        break
      fi
      previous="$arg"
    done
  fi
  session_id="${AGENT_SESSION_FAKE_CODEX_SESSION_ID:-fake-codex-session}"
  timestamp="${AGENT_SESSION_FAKE_CODEX_SESSION_TIMESTAMP:-2099-01-01T00:00:00Z}"
  old_ifs="$IFS"
  IFS=":"
  index=1
  for file in $AGENT_SESSION_FAKE_CODEX_SESSION_FILE; do
    IFS="$old_ifs"
    current_id="$session_id"
    if [ $index -gt 1 ]; then
      current_id="${session_id}-${index}"
    fi
    mkdir -p "$(dirname "$file")"
    if [ "${AGENT_SESSION_FAKE_CODEX_APPEND:-0}" = "1" ]; then
      printf '{"type":"event","timestamp":"%s"}\n' "$timestamp" >> "$file"
    else
      printf '{"timestamp":"%s","type":"session_meta","payload":{"id":"%s","session_id":"%s","cwd":"%s","source":"cli","timestamp":"%s"}}\n' "$timestamp" "$current_id" "$current_id" "$cwd" "$timestamp" > "$file"
    fi
    IFS=":"
    index=$((index + 1))
  done
  IFS="$old_ifs"
fi

if [ "$1" = "new-session" ] && [ "${AGENT_SESSION_FAKE_RECORD_AT_NEW_SESSION+x}" = "x" ]; then
  if ! grep -Eq '"generation"[[:space:]]*:[[:space:]]*2' "$AGENT_SESSION_FAKE_RECORD_AT_NEW_SESSION"; then
    printf 'runtime generation was not persisted before tmux launch\n' >&2
    exit 43
  fi
fi

if [ "$1" = "new-session" ] && [ "${AGENT_SESSION_FAKE_CHMOD_AFTER_NEW_SESSION+x}" = "x" ]; then
  chmod "${AGENT_SESSION_FAKE_CHMOD_MODE:-0500}" "$AGENT_SESSION_FAKE_CHMOD_AFTER_NEW_SESSION"
fi

if [ "$1" = "new-session" ] && [ "${AGENT_SESSION_FAKE_MKDIR_AFTER_NEW_SESSION+x}" = "x" ]; then
  mkdir -p "$AGENT_SESSION_FAKE_MKDIR_AFTER_NEW_SESSION"
fi

if [ "$1" = "new-session" ]; then
  : > "$AGENT_SESSION_FAKE_TMUX_LOG.launched"
  if [ "${AGENT_SESSION_FAKE_TMUX_MALFORMED_LAUNCH_IDENTITY:-0}" = "1" ]; then
    printf 'contaminated launch output\n'
    exit 0
  fi
  tmux_name=""
  launch_agent_session_id=""
  launch_state_dir=""
  launch_runtime_id=""
  previous=""
  for arg in "$@"; do
    if [ "$previous" = "-s" ]; then
      tmux_name="$arg"
    elif [ "$previous" = "-e" ]; then
      case "$arg" in
        AGENT_SESSION_ID=*) launch_agent_session_id="${arg#AGENT_SESSION_ID=}" ;;
        AGENT_SESSION_STATE_DIR=*) launch_state_dir="${arg#AGENT_SESSION_STATE_DIR=}" ;;
        AGENT_SESSION_RUNTIME_ID=*) launch_runtime_id="${arg#AGENT_SESSION_RUNTIME_ID=}" ;;
      esac
    fi
    previous="$arg"
  done
  identity_number=77
  while ! ( set -C; : > "$AGENT_SESSION_FAKE_TMUX_LOG.identity-$identity_number" ) 2>/dev/null; do
    identity_number=$((identity_number + 1))
  done
  session_identity="${AGENT_SESSION_FAKE_TMUX_SESSION_ID:-}"
  [ -n "$session_identity" ] || session_identity="\$$identity_number"
  pane_identity="${AGENT_SESSION_FAKE_TMUX_PANE_ID:-}"
  [ -n "$pane_identity" ] || pane_identity="%$identity_number"
  pane_pid="${AGENT_SESSION_FAKE_TMUX_PANE_PID:-}"
  if [ -n "${AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID:-}" ] &&
     [ -n "$launch_agent_session_id" ] &&
     [ "$launch_agent_session_id" != "$AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID" ]; then
    pane_pid=""
  fi
  if [ -z "$pane_pid" ]; then
    start_live_pane || exit 1
    pane_pid="$NILS_TEST_PANE_PID"
  fi
  pane_start_time="$(pane_process_start_time "$pane_pid")"
  [ -n "$pane_start_time" ] || exit 1
  printf '%s\t%s\t%s\t%s\t%s\n' "$tmux_name" "$session_identity" "$pane_identity" "$pane_pid" "$pane_start_time" >> "$AGENT_SESSION_FAKE_TMUX_LOG.panes"
  if [ -n "$tmux_name" ]; then
    printf '%s\t%s\t%s\t%s\n' "$tmux_name" "$session_identity" "$pane_identity" "$pane_pid" >> "$AGENT_SESSION_FAKE_TMUX_LOG.identities"
    printf '%s\t%s\t%s\n' "$tmux_name" "AGENT_SESSION_ID" "$launch_agent_session_id" >> "$AGENT_SESSION_FAKE_TMUX_LOG.environments"
    printf '%s\t%s\t%s\n' "$tmux_name" "AGENT_SESSION_STATE_DIR" "$launch_state_dir" >> "$AGENT_SESSION_FAKE_TMUX_LOG.environments"
    printf '%s\t%s\t%s\n' "$tmux_name" "AGENT_SESSION_RUNTIME_ID" "$launch_runtime_id" >> "$AGENT_SESSION_FAKE_TMUX_LOG.environments"
  fi
  heartbeat=""
  heartbeat_slot=0
  for arg in "$@"; do
    if [ "$heartbeat_slot" = "2" ]; then
      incarnation="$arg"
      break
    fi
    if [ "$heartbeat_slot" = "1" ]; then
      heartbeat_slot=2
      continue
    fi
    case "$arg" in
      */coordination/heartbeat) heartbeat="$arg"; heartbeat_slot=1 ;;
    esac
  done
  if [ -n "$heartbeat" ] && [ -n "${incarnation:-}" ]; then
    mkdir -p "$(dirname "$heartbeat")"
    printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"
    chmod 600 "$heartbeat"
  fi
  printf '%s\t%s\t%s\n' "$session_identity" "$pane_identity" "$pane_pid"
fi

if [ "$1" = "send-keys" ] && [ "${AGENT_SESSION_FAKE_TMUX_ENTER_HOOK+x}" = "x" ]; then
  last_arg=""
  for arg in "$@"; do
    last_arg="$arg"
  done
  if [ "$last_arg" = "Enter" ]; then
    count_file="${AGENT_SESSION_FAKE_TMUX_ENTER_COUNT_FILE:?}"
    count=0
    if [ -r "$count_file" ]; then
      count="$(cat "$count_file")"
    fi
    count=$((count + 1))
    printf '%s\n' "$count" > "$count_file"
    if [ "$count" -eq "${AGENT_SESSION_FAKE_TMUX_ENTER_HOOK_AT:-2}" ]; then
      "$AGENT_SESSION_FAKE_TMUX_ENTER_HOOK" &
    fi
  fi
fi

if [ "$1" = "send-keys" ] && [ -n "${AGENT_SESSION_FAKE_TMUX_SEND_KEYS_SLEEP:-}" ]; then
  sleep "$AGENT_SESSION_FAKE_TMUX_SEND_KEYS_SLEEP"
fi

if [ "${AGENT_SESSION_FAKE_TMUX_FAIL_AFTER:-}" = "$operation" ]; then
  fail_once_dir="${AGENT_SESSION_FAKE_TMUX_FAIL_AFTER_ONCE_DIR:-}"
  if [ -z "$fail_once_dir" ] || mkdir "$fail_once_dir" 2>/dev/null; then
    echo "fake tmux failed after $operation" >&2
    exit 42
  fi
fi

exit 0
"##
        .replace("__NILS_TEST_PANE_PARENT__", &pane_parent)
        .replace("__NILS_TEST_PANE_START_TIME__", pane_start_time),
    );
    // Pre-warm the fixture: the first exec of a freshly written script pays a
    // host EDR/filesystem cold-start tail that can exceed the product's bounded
    // probe budget, turning the intended probe/kill outcome into
    // verification-failed (sympoies/nils-cli#2131). Exec it once here so the
    // product's first exec runs warm. `version` matches no dispatch branch, so
    // the only side effects are one logged record and the logger's lock file;
    // remove both so the test observes exactly the product's calls, with no
    // pre-existing log file to observe.
    let prewarm = prewarm_fake_tmux(&bin, &log);
    assert!(
        prewarm.status.success(),
        "fake tmux prewarm failed: {}",
        String::from_utf8_lossy(&prewarm.stderr)
    );
    fs::remove_file(&log).expect("clear fake tmux prewarm record");
    fs::remove_file(log.with_extension("log.call-lock")).expect("clear fake tmux prewarm lock");
    (bin, log)
}

#[test]
fn guarded_teardown_rejects_mismatched_identity_without_signaling() {
    use std::process::Command;
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, log) = fake_tmux(tmp.path());
    let child = super::spawn_test_process_group();
    let pid = child.pid();
    let expected = guarded_arguments(pid);
    let mut malformed = Vec::new();
    for (index, value) in [
        (3, "%78".to_string()),
        (4, expected[4].replace("$77", "$78")),
        (
            4,
            expected[4].replace(
                &format!("pane_pid}},{pid}"),
                &format!("pane_pid}},{}", pid + 1),
            ),
        ),
        (
            5,
            expected[5].replace(&format!("-- -{pid}"), &format!("-- -{}", pid + 1)),
        ),
        (5, expected[5].replace("$77", "$78")),
        (5, "run-shell \"exit 0\"; kill-session -t $77".to_string()),
    ] {
        let mut args = expected.clone();
        args[index] = value;
        malformed.push(args);
    }
    for args in malformed {
        let output = Command::new(&tmux)
            .args(args)
            .env("AGENT_SESSION_FAKE_TMUX_LOG", &log)
            .env("AGENT_SESSION_FAKE_TMUX_PANE_PID", pid.to_string())
            .env("AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID", pid.to_string())
            .env("PYTHONOPTIMIZE", "1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            unsafe { libc::kill(pid as libc::pid_t, 0) },
            0,
            "owned pane stays live"
        );
        assert!(
            !log.with_extension("log.killed").exists(),
            "no session removal"
        );
    }
}

#[test]
fn guarded_teardown_signals_only_the_expected_group_and_removes_the_session() {
    use std::process::Command;
    use std::time::{Duration, Instant};
    let tmp = tempfile::TempDir::new().unwrap();
    let (tmux, log) = fake_tmux(tmp.path());
    let child = super::spawn_test_process_group();
    let pid = child.pid();
    let output = Command::new(&tmux)
        .args(guarded_arguments(pid))
        .env("AGENT_SESSION_FAKE_TMUX_LOG", &log)
        .env("AGENT_SESSION_FAKE_TMUX_PANE_PID", pid.to_string())
        .env("AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID", pid.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(log.with_extension("log.killed")).unwrap(),
        "$77\n"
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let result = unsafe { libc::kill(-(pid as libc::pid_t), 0) };
        let errno = (result != 0)
            .then(|| std::io::Error::last_os_error().raw_os_error())
            .flatten();
        if result == -1 && errno == Some(libc::ESRCH) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "owned group must exit after SIGKILL"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn guarded_arguments(pid: u32) -> Vec<String> {
    let pid_clause = format!("#{{==:#{{pane_pid}},{pid}}}");
    let pane_clause = format!("#{{&&:#{{==:#{{pane_id}},%77}},{pid_clause}}}");
    let identity = format!("#{{&&:#{{==:#{{session_id}},$77}},{pane_clause}}}");
    vec![
        "if-shell".into(),
        "-F".into(),
        "-t".into(),
        "%77".into(),
        format!("#{{&&:#{{==:#{{pane_dead}},0}},{identity}}}"),
        format!("run-shell \"/bin/kill -KILL -- -{pid}\"; kill-session -t $77"),
        "display-message -p agent-session-runtime-identity-changed".into(),
    ]
}
