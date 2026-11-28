"""Measure a real daemon over synthetic records with an argv-counting fake tmux."""
import argparse
from collections import Counter
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("--seconds", type=int, default=60)
parser.add_argument("--warmup", type=int, default=20)
parser.add_argument("--sessions", type=int, default=20)
parser.add_argument("--max-starts", type=int)
args = parser.parse_args()

def stamp(age=0):
    return (datetime.now(timezone.utc) - timedelta(seconds=age)).isoformat()

def cpu(pid):
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK")

with tempfile.TemporaryDirectory(prefix="poll-", dir=os.environ.get("TMPDIR")) as tmp:
    tmp = Path(tmp)
    state, home, tools = (tmp / name for name in ("state", "home", "bin"))
    for path in (state, home, tools):
        path.mkdir(mode=0o700)
    fixture_names = [f"fixture-{i:02d}" for i in range(args.sessions)]
    now, old = stamp(), stamp(600)
    for i, name in enumerate(fixture_names):
        provider = "codex" if i % 2 else "claude"
        directory = state / "sessions" / name
        directory.mkdir(parents=True)
        (directory / "session.json").write_text(json.dumps({
            "schema_version": "agent-session.session.v1",
            "id": name, "agent": provider, "mode": "interactive", "cwd": str(home),
            "tmux_session": name, "title": None, "prompt_file": None, "log_file": None,
            "created_at": old, "updated_at": old,
            "runtime": {"kind": "tmux", "tmux_session": name, "launch_id": f"runtime-{i:02d}",
                        "generation": 1, "started_at": old},
        }))
        (directory / "activity.json").write_text(json.dumps({
            "schema_version": "agent-session.activity.v1",
            "runtime_id": f"runtime-{i:02d}", "runtime_generation": 1,
            "state": {"schema_version": "agent-session.turn-state.v1", "phase": "unknown",
                      "phase_changed_at": old, "revision": 1,
                      "source": {"kind": "terminal_heuristic", "confidence": "inferred"}},
        }))
    calls = tmp / "tmux-calls.tsv"
    calls.touch()
    fake_tmux = tools / "tmux"
    fake_tmux.write_text(r'''#!/bin/sh
printf '%s\t%s\n' "$1" "$*" >> "$FIXTURE_CALLS"
case "$1" in
list-windows) /bin/cat "$FIXTURE_WINDOWS" ;;
list-panes) /bin/cat "$FIXTURE_PANES" ;;
list-sessions) /bin/cat "$FIXTURE_SESSIONS" ;;
has-session) exit 0 ;;
show-environment)
    key=""
    target=""
    for arg; do
        case "$arg" in
            fixture-*) target="$arg" ;;
            AGENT_SESSION_*) key="$arg" ;;
        esac
    done
    case "$key" in
        AGENT_SESSION_ID) printf '%s=%s\n' "$key" "$target" ;;
        AGENT_SESSION_STATE_DIR) printf '%s=%s\n' "$key" "$FIXTURE_STATE" ;;
        AGENT_SESSION_RUNTIME_ID) printf '%s=runtime-%s\n' "$key" "${target#fixture-}" ;;
        *) exit 1 ;;
    esac ;;
display-message)
    for last; do :; done
    case "$last" in
        '#{pane_title}') printf '%s\n' 'fixture idle' ;;
        '#{window_activity}') printf '%s\n' "$FIXTURE_EPOCH" ;;
        *) exit 1 ;;
    esac ;;
capture-pane) printf '%s\n' '› Explain this codebase' ;;
*) exit 1 ;;
esac
''')
    fake_tmux.chmod(0o700)
    fake_hostname = tools / "hostname"
    fake_hostname.write_text(
        '#!/bin/sh\nprintf "hostname\\n" >> "$FIXTURE_HOST_CALLS"\nprintf "fixture-host\\n"\n'
    )
    fake_hostname.chmod(0o700)
    for name in ["claude", "codex", "claude-cli", "codex-cli", "agent-hook", "retitle"]:
        path = tools / name
        path.write_text("#!/bin/sh\nexit 1\n")
        path.chmod(0o700)
    epoch = str(int(time.time()) - 600)
    files = {
        "WINDOWS": [f"{name}|{epoch}" for name in fixture_names],
        "PANES": [f"{name}|{epoch}|%{i}|fixture idle" for i, name in enumerate(fixture_names)],
        "SESSIONS": fixture_names,
    }
    for key, lines in files.items():
        (tmp / key).write_text("\n".join(lines) + "\n")
    host_calls = tmp / "hostname-calls.txt"
    host_calls.touch()
    retitle_cfg = json.dumps({"provider": "command", "argv": [str(tools / "retitle")]})
    env = {
        "HOME": str(home), "PATH": f"{tools}:/usr/bin:/bin", "LANG": "C.UTF-8",
        "TMPDIR": str(tmp), "TOKIO_WORKER_THREADS": "2",
        "AGENT_SESSION_STATE_DIR": str(state),
        "AGENT_SESSION_TMUX_BIN": str(fake_tmux),
        "AGENT_SESSION_CLAUDE_BIN": str(tools / "claude"),
        "AGENT_SESSION_CODEX_BIN": str(tools / "codex"),
        "AGENT_SESSION_RETITLE_CONFIG": retitle_cfg,
        "FIXTURE_CALLS": str(calls), "FIXTURE_STATE": str(state), "FIXTURE_EPOCH": epoch,
        "FIXTURE_HOST_CALLS": str(host_calls),
        **{f"FIXTURE_{key}": str(tmp / key) for key in files},
    }
    with (tmp / "serve.log").open("wb") as log:
        proc = subprocess.Popen([str(args.binary.resolve()), "serve", "--bind", "127.0.0.1:0",
                                 "--machine", "fixture-machine"], env=env,
                                stdout=log, stderr=log)
        try:
            endpoint = state / "coordination" / "daemon-endpoint.json"
            deadline = time.monotonic() + 8
            while not endpoint.is_file():
                if proc.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError((tmp / "serve.log").read_text())
                time.sleep(0.05)
            time.sleep(args.warmup)
            offset = calls.stat().st_size
            before = cpu(proc.pid)
            started = time.monotonic()
            time.sleep(args.seconds)
            elapsed = time.monotonic() - started
            consumed = cpu(proc.pid) - before
            with calls.open() as stream:
                stream.seek(offset)
                counted = Counter(line.split("\t", 1)[0] for line in stream if "\t" in line)
            total = counted.total()
            print(json.dumps({
                "sessions": args.sessions, "elapsed_seconds": round(elapsed, 2),
                "daemon_cpu_seconds": round(consumed, 3),
                "daemon_cpu_percent_of_one_core": round(consumed / elapsed * 100, 2),
                "tmux_execs_by_kind": dict(sorted(counted.items())),
                "tmux_execs_per_second_by_kind": {
                    key: round(value / elapsed, 4) for key, value in sorted(counted.items())
                },
                "tmux_total": total, "tmux_execs_per_minute": round(total / elapsed * 60, 2),
                "tmux_execs_per_idle_session_minute": round(total / elapsed * 60 / args.sessions, 3),
                "hostname_execs_at_startup": len(host_calls.read_text().splitlines()),
            }, indent=2), flush=True)
            if args.max_starts is not None:
                assert total <= args.max_starts, f"expected at most {args.max_starts} tmux starts; observed {total}"
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
