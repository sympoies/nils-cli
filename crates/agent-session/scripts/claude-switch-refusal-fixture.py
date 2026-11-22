"""Run an isolated Claude account-switch refusal against an installed binary."""

import argparse
import http.client
import json
import os
import re
import secrets
import select
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SESSION = "claude-switch-fixture"
SOURCE = "fixture-source"
TARGET = "fixture-target"
BROKER_SCHEMA = "agent-session.account-broker.v2"
COUNTS = {
    "private_roots": 1,
    "serve_daemons": 1,
    "tmux_servers": 1,
    "managed_sessions": 1,
    "synthetic_provider_processes": 1,
    "account_nicknames": 2,
    "materialized_account_stores": 1,
}
REFUSAL = {
    "http_status": 422,
    "code": "claude-account-switch-refused",
    "cause": "claude-account-broker-rejected",
}
PRIVATE_PATH = "/usr/bin:/bin"


class FixtureError(Exception):
    def __init__(self, reason, status="FAIL"):
        super().__init__(reason)
        self.reason = reason
        self.status = status


def require(condition, reason):
    if not condition:
        raise FixtureError(reason)


def command(argv, env, *, input_text=None, timeout=20):
    return subprocess.run(
        argv, env=env, input=input_text, text=True, capture_output=True,
        timeout=timeout, check=False,
    )


def private_file(path, content, mode=0o600):
    with os.fdopen(
        os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode), "w",
    ) as stream:
        stream.write(content)


def executable(name):
    path = shutil.which(name)
    return Path(path).resolve() if path else None


def prerequisites(binary):
    if not binary:
        raise FixtureError("agent-session-unavailable", "UNRUN")
    if sys.platform != "linux" or not hasattr(os, "pidfd_open"):
        raise FixtureError("linux-pidfd-unavailable", "UNRUN")
    try:
        fd = os.pidfd_open(os.getpid())
        os.close(fd)
    except OSError as error:
        raise FixtureError("linux-pidfd-unavailable", "UNRUN") from error
    if not Path("/sys/fs/cgroup/cgroup.controllers").is_file():
        raise FixtureError("cgroup-v2-unavailable", "UNRUN")
    tmux = executable("tmux")
    if not tmux:
        raise FixtureError("tmux-unavailable", "UNRUN")
    version = command([binary, "--version"], {"PATH": PRIVATE_PATH}, timeout=5)
    match = re.match(r"agent-session (1)\.(\d+)\.(\d+)\b", version.stdout)
    if version.returncode or not match or tuple(map(int, match.groups())) < (1, 41, 10):
        raise FixtureError("agent-session-version-unsupported", "UNRUN")
    if not all(shutil.which(tool, path=PRIVATE_PATH) for tool in ("systemctl", "systemd-run")):
        raise FixtureError("systemd-user-unavailable", "UNRUN")
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR") or f"/run/user/{os.geteuid()}")
    try:
        sockets = {name: (runtime / name).resolve(strict=True)
                   for name in ("bus", "systemd/private")}
        for socket in sockets.values():
            info = socket.stat()
            if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.geteuid():
                raise OSError("user manager socket unavailable")
    except OSError as error:
        raise FixtureError("systemd-user-unavailable", "UNRUN") from error
    return tmux, version.stdout.split()[1], sockets


def poll_until(read, reason, seconds=5):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = read()
        if value:
            return value
        time.sleep(0.05)
    raise FixtureError(reason)


def exited(fd):
    poller = select.poll()
    poller.register(fd, select.POLLIN)
    return bool(poller.poll(0))


class Fixture:
    def __init__(self, binary, tmux, sockets):
        self.root = Path(tempfile.mkdtemp(prefix="claude-switch-", dir="/tmp"))
        self.state = self.root / "state"
        self.runtime = self.root / "run"
        self.store = self.root / "accounts"
        self.tmux = self.root / "tmux"
        self.cli = [binary, "--state-dir", self.state]
        self.tmux_binary = tmux
        self.sockets = sockets
        self.daemon = None
        self.create_attempted = False
        self.process_fds = []
        self.scope_events = set()
        self.token = secrets.token_urlsafe(32)
        self.env = {
            "PATH": PRIVATE_PATH,
            "HOME": str(self.root / "home"),
            "AGENT_HOME": str(self.root / "agent-home"),
            "XDG_RUNTIME_DIR": str(self.runtime),
            "TMPDIR": str(self.root / "tmp"),
            "AGENT_SESSION_TMUX_BIN": str(self.tmux),
            "AGENT_SESSION_TMUX_SCOPE": "1",
            "AGENT_SESSION_CLAUDE_BIN": str(self.root / "claude"),
            "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER": json.dumps([str(self.root / "broker")]),
        }

    def prepare(self):
        for directory in ("home", "agent-home", "state", "run", "run/systemd", "work", "tmp",
                          "accounts", f"accounts/{SOURCE}"):
            (self.root / directory).mkdir(mode=0o700)
        for name, socket in self.sockets.items():
            (self.runtime / name).symlink_to(socket)
        manager = command(["systemctl", "--user", "show", "-p", "Version"],
                          self.env, timeout=5)
        if manager.returncode:
            raise FixtureError("systemd-user-unavailable", "UNRUN")

        config = self.store / SOURCE
        # Claude account admission checks that the file exists and is private to
        # the fixture owner. The synthetic process never opens it.
        private_file(config / ".credentials.json", "{}\n")
        private_file(self.root / "token", self.token + "\n")
        private_file(self.root / "target-attempts", "")
        private_file(self.root / "provider-launches", "")
        scope_socket = shlex.quote(str(self.root / "tmux.sock"))
        private_file(self.tmux, f"""#!/bin/sh
exec {shlex.quote(str(self.tmux_binary))} -S {scope_socket} -f /dev/null "$@"
""", 0o700)

        launches = shlex.quote(str(self.root / "provider-launches"))
        private_file(self.root / "claude", f"""#!/bin/sh
while [ "$#" -gt 1 ]; do
    case "$1" in
        --session-id|--resume)
            printf '%s %s\n' "$2" "$$" >> {launches}
            exec /bin/sleep 120
            ;;
    esac
    shift
done
exit 64
""", 0o700)

        inventory = json.dumps({
            "schema_version": BROKER_SCHEMA,
            "provider": "claude",
            "accounts": [{"account": SOURCE}, {"account": TARGET}],
        })
        source_response = json.dumps({
            "schema_version": BROKER_SCHEMA, "provider": "claude", "account": SOURCE,
            "config_dir": str(config),
        })
        attempts = shlex.quote(str(self.root / "target-attempts"))
        private_file(self.root / "broker", f"""#!/bin/sh
case "$*" in
    'list --provider claude --format json')
        printf '%s\n' {shlex.quote(inventory)}
        ;;
    'materialize --provider claude --account {SOURCE} --format json')
        printf '%s\n' {shlex.quote(source_response)}
        ;;
    'materialize --provider claude --account {TARGET} --format json')
        printf x >> {attempts}
        exit 3
        ;;
    *) exit 64 ;;
esac
""", 0o700)

    def start(self):
        self.daemon = subprocess.Popen(
            [*self.cli, "serve", "--bind", "127.0.0.1:0", "--machine", "claude-switch-fixture",
             "--token-stdin"],
            env=self.env, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        with self.daemon.stdin as stream:
            stream.write((self.token + "\n").encode())

        endpoint = self.state / "coordination" / "daemon-endpoint.json"

        def read_endpoint():
            require(self.daemon.poll() is None, "serve-start-failed")
            if endpoint.is_file():
                return json.loads(endpoint.read_text())["url"]
            return None

        self.url = poll_until(read_endpoint, "serve-readiness-timeout")
        match = re.fullmatch(r"http://127\.0\.0\.1:([1-9]\d{0,4})", self.url)
        require(match and int(match[1]) <= 65535, "serve-endpoint-invalid")
        self.port = int(match[1])
        self.create_attempted = True
        status, payload = self.request("POST", "/sessions", {
            "id": SESSION, "agent": "claude", "cwd": str(self.root / "work"),
            "title": "Claude switch refusal fixture", "claude_account": SOURCE,
        })
        require(status == 200, "session-create-failed")
        session = payload["data"]["session"]
        self.incarnation = session["session_incarnation"]
        self.conversation = session["provider_resume"]["session_id"]
        poll_until(lambda: (self.root / "provider-launches").stat().st_size,
                   "synthetic-provider-start-timeout")
        self.pin_processes(self.snapshot())

        for kind in ("turn_started", "turn_completed"):
            event = {
                "schema_version": "agent-session.turn-event.v1", "event_id": kind,
                "runtime_id": self.incarnation, "provider": "claude",
                "provider_session_id": self.conversation, "provider_turn_id": "fixture-turn",
                "kind": kind, "confidence": "observed", "source_kind": "console_observation",
            }
            ingested = command(
                [*self.cli, "activity", "event", SESSION, "--stdin", "--format", "json"],
                self.env, input_text=json.dumps(event), timeout=5,
            )
            require(ingested.returncode == 0, "activity-event-refused")

    def request(self, method, path, body=None):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=20)
        try:
            connection.request(method, path, None if body is None else json.dumps(body), {
                "Authorization": "Bearer " + self.token,
                "Content-Type": "application/json",
            })
            response = connection.getresponse()
            return response.status, json.loads(response.read(1 << 20))
        finally:
            connection.close()

    def pin_processes(self, snapshot):
        def cgroup(pid):
            for line in Path(f"/proc/{pid}/cgroup").read_text().splitlines():
                if line.startswith("0::/"):
                    return Path(line[3:])
            raise FixtureError("runtime-cgroup-unavailable")

        caller_scope = cgroup(os.getpid())
        # The runtime can place the tmux server and pane in separate scopes.
        for key in ("provider_pid", "pane_pid", "tmux_pid"):
            pid = snapshot[key]
            scope = cgroup(pid)
            require(scope.name.endswith(".scope") and scope != caller_scope,
                    "fixture-scope-unavailable")
            events = Path("/sys/fs/cgroup") / scope.relative_to("/") / "cgroup.events"
            require(events.is_file(), "fixture-scope-unavailable")
            self.scope_events.add(events)
            self.process_fds.append(os.pidfd_open(pid))

    def snapshot(self):
        status, payload = self.request("GET", "/sessions")
        require(status == 200, "session-readback-failed")
        sessions = payload["data"]["sessions"]
        require(len(sessions) == 1 and sessions[0]["id"] == SESSION, "session-readback-mismatch")
        row = sessions[0]
        require(row["agent"] == row["provider_resume"]["provider"] == "claude",
                "provider-readback-mismatch")
        account = row["claude_account"]
        panes = command(
            [self.tmux, "list-panes", "-a", "-F", "#{session_name} #{pane_id} #{pane_pid} #{pid}"],
            self.env, timeout=5,
        )
        rows = panes.stdout.splitlines()
        require(panes.returncode == 0 and len(rows) == 1, "tmux-readback-mismatch")
        tmux_session, pane_id, pane_pid, tmux_pid = rows[0].split()
        require(tmux_session == row["tmux_session"], "tmux-readback-mismatch")
        launches = (self.root / "provider-launches").read_text().splitlines()
        require(len(launches) == 1, "synthetic-provider-launch-count-changed")
        conversation, provider_pid = launches[0].split()
        require(row["provider_resume"]["session_id"] == conversation == self.conversation,
                "provider-session-identity-mismatch")
        return {
            "id": row["id"], "status": row["status"], "turn_phase": row["turn_state"]["phase"],
            "incarnation": row["session_incarnation"], "provider_session_id": conversation,
            "provider_pid": int(provider_pid), "provider_launches": len(launches),
            "pane_id": pane_id, "pane_pid": int(pane_pid), "tmux_pid": int(tmux_pid),
            "account": {
                "selected": account["selected_account"], "state": account["state"],
                "revision": account["revision"], "applied_runtime_id": account["applied_runtime_id"],
                "next": account.get("next"),
            },
        }

    def account_inventory(self):
        status, payload = self.request("GET", "/claude/accounts")
        require(status == 200, "account-inventory-readback-failed")
        accounts = payload["data"]["accounts"]
        require({account["account"] for account in accounts} == {SOURCE, TARGET}
                and len(accounts) == 2, "account-inventory-mismatch")
        return accounts

    def store_metadata(self):
        entries = [self.store, *sorted(self.store.rglob("*"))]
        metadata = []
        for path in entries:
            info = path.lstat()
            metadata.append((path.relative_to(self.store), info.st_dev, info.st_ino, info.st_mode,
                             info.st_uid, info.st_size, info.st_mtime_ns, info.st_ctime_ns))
        return metadata

    def consumer(self, argv):
        env = {
            "PATH": PRIVATE_PATH, "HOME": self.env["HOME"], "TMPDIR": self.env["TMPDIR"],
            "AGENT_SESSION_FIXTURE_URL": self.url,
            "AGENT_SESSION_FIXTURE_TOKEN_FILE": str(self.root / "token"),
            "AGENT_SESSION_FIXTURE_SESSION_ID": SESSION,
            "AGENT_SESSION_FIXTURE_INCARNATION": self.incarnation,
            "AGENT_SESSION_FIXTURE_TARGET_ACCOUNT": TARGET,
        }
        child = subprocess.Popen(
            argv, cwd=self.root / "work", env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        try:
            require(child.wait(timeout=30) == 0, "consumer-failed")
        except subprocess.TimeoutExpired as error:
            raise FixtureError("consumer-timeout") from error
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)

    def exercise(self, report, consumer):
        before = self.snapshot()
        require(before["status"] == "running" and before["turn_phase"] == "waiting"
                and before["account"]["state"] == "bound"
                and before["account"]["selected"] == SOURCE and before["account"]["next"] is None
                and before["incarnation"] == before["account"]["applied_runtime_id"]
                and not any(map(exited, self.process_fds)), "fixture-not-ready")
        inventory = self.account_inventory()
        store = self.store_metadata()
        code, body = self.request("PUT", f"/sessions/{SESSION}/account", {
            "account": TARGET, "expected_session_incarnation": self.incarnation,
        })
        error = body.get("error", {})
        refusal = {
            "http_status": code, "code": error.get("code"),
            "cause": error.get("details", {}).get("cause"),
        }
        require(refusal == REFUSAL, "unexpected-switch-result")
        if consumer:
            self.consumer(consumer)
        attempts = (self.root / "target-attempts").stat().st_size
        require(attempts == (2 if consumer else 1), "target-attempt-count-mismatch")
        after = self.snapshot()
        store_unchanged = self.store_metadata() == store and self.account_inventory() == inventory
        report.update(refusal=refusal, before=before, after=after,
                      account_store_unchanged=store_unchanged,
                      target_materialization_attempts=attempts)
        require(after == before and store_unchanged and not any(map(exited, self.process_fds)),
                "switch-mutated-fixture")

    def cleanup(self, report):
        if self.daemon and self.daemon.poll() is None:
            self.daemon.terminate()
            try:
                self.daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.daemon.kill()
                self.daemon.wait(timeout=5)
        if self.create_attempted:
            deleted = command([*self.cli, "delete", SESSION, "--format", "json"], self.env)
            if deleted.returncode:
                require(json.loads(deleted.stdout)["error"]["code"] == "session-not-found",
                        "owned-session-delete-failed")
            result = command([*self.cli, "list", "--format", "json"], self.env, timeout=5)
            require(result.returncode == 0 and json.loads(result.stdout)["data"] == [],
                    "owned-session-still-listed")
            status = command([self.tmux, "list-sessions"], self.env, timeout=5)
            require(status.returncode != 0 and (
                status.stderr.startswith("no server running on ")
                or status.stderr.endswith("(No such file or directory)\n")
            ), "owned-tmux-still-running")
        def processes_absent():
            if not all(map(exited, self.process_fds)):
                return False
            for events in self.scope_events:
                try:
                    if "populated 0" not in events.read_text().splitlines():
                        return False
                except FileNotFoundError:
                    pass
            return True

        try:
            poll_until(processes_absent, "owned-processes-still-running")
        finally:
            for fd in self.process_fds:
                os.close(fd)
        shutil.rmtree(self.root)
        require(not self.root.exists(), "owned-root-still-present")
        report["cleanup"] = {
            "daemon_absent": True, "session_absent": True, "tmux_absent": True,
            "provider_absent": True, "root_absent": True,
        }


def interrupted(_signum, _frame):
    raise FixtureError("interrupted")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent-session", required=True, help="installed agent-session executable")
    parser.add_argument("--dry-run", action="store_true", help="print exact planned resource counts")
    parser.add_argument("consumer", nargs=argparse.REMAINDER,
                        help="optional foreground consumer command after --")
    args = parser.parse_args()
    consumer = args.consumer[1:] if args.consumer[:1] == ["--"] else args.consumer
    report = {"schema_version": "agent-session.claude-switch-refusal-fixture.v1"}
    if args.dry_run:
        print(json.dumps({**report, "status": "DRY_RUN", "counts": COUNTS}))
        return 0

    fixture = None
    for signum in (signal.SIGINT, signal.SIGTERM):
        signal.signal(signum, interrupted)
    try:
        if consumer:
            consumer[0] = executable(consumer[0])
            if not consumer[0]:
                raise FixtureError("consumer-unavailable", "UNRUN")
        binary = executable(args.agent_session)
        tmux, version, sockets = prerequisites(binary)
        report["runtime_version"] = version
        fixture = Fixture(binary, tmux, sockets)
        fixture.prepare()
        fixture.start()
        fixture.exercise(report, consumer)
        report["status"] = "PASS"
    except FixtureError as error:
        report.update(status=error.status, reason=error.reason)
    except (OSError, subprocess.SubprocessError, http.client.HTTPException, ValueError, KeyError):
        report.update(status="FAIL", reason="fixture-operation-failed")
    finally:
        if fixture:
            for signum in (signal.SIGINT, signal.SIGTERM):
                signal.signal(signum, signal.SIG_IGN)
            try:
                fixture.cleanup(report)
            except (FixtureError, OSError, subprocess.SubprocessError, ValueError, KeyError):
                report.update(status="FAIL", reason="cleanup-incomplete",
                              recovery_root=str(fixture.root))
    print(json.dumps(report))
    return {"PASS": 0, "FAIL": 1, "UNRUN": 2}[report["status"]]


if __name__ == "__main__":
    sys.exit(main())
