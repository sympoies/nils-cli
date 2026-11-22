"""Worker for the native notice acceptance sandbox."""

import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

import native_notice_inference as inference


ROOT = Path("/f")
CONTROL = "http://127.0.0.1:8781"
AGENTS = {"claude": "claude_mod", "codex": "codex_app_server"}
SEED = "Synthetic fixture seed."
NOTICE_BODY = "Synthetic native notice."
KIND = "acceptance.fixture"
SOURCE = "synthetic"
HEADER = f"<!-- agent-notice:v1 kind={KIND} source={SOURCE} -->\n\n"
NOTICE = HEADER + NOTICE_BODY
LOOKALIKE = HEADER + "Synthetic user lookalike."
TOKEN = "synthetic-local-fixture-operator"
NOTICE_ARGS = ("--kind", KIND, "--source", SOURCE, "--text-stdin")


class Refused(Exception):
    def __init__(self, code):
        super().__init__(code)
        self.code = code


def unrun(code):
    return {"status": "UNRUN", "code": code}


def run_cli(*args, text=None):
    try:
        result = subprocess.run(
            ["agent-session", *args, "--format", "json"],
            input=text, text=True, capture_output=True, timeout=25,
        )
    except subprocess.TimeoutExpired as exc:
        raise Refused("cli-attempt-timeout") from exc
    return checked(result.stdout, result.returncode == 0)


def checked(raw, success):
    try:
        value = json.loads(raw)
    except ValueError as exc:
        raise Refused("invalid-control-response") from exc
    if not isinstance(value, dict):
        raise Refused("invalid-control-response")
    if success and value.get("ok") is True:
        return value["data"]
    error = value.get("error")
    code = error.get("code", "") if isinstance(error, dict) else ""
    if not isinstance(code, str) or not re.fullmatch(r"[a-z][a-z0-9-]{0,79}", code):
        code = "invalid-control-response"
    raise Refused(code)


def request(path, body=None, timeout=3):
    data = None if body is None else json.dumps(body).encode()
    req = Request(CONTROL + path, data, {
        "Authorization": f"Bearer {TOKEN}", "Content-Type": "application/json",
    })
    try:
        with urlopen(req, timeout=timeout) as response:
            return checked(response.read(2**20), True)
    except HTTPError as exc:
        with exc:
            return checked(exc.read(2**20), False)
    except (URLError, TimeoutError, ConnectionError) as exc:
        raise Refused("control-unavailable") from exc


def wait_for(probe, seconds, code):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        result = probe()
        if result is not None:
            return result
        time.sleep(0.2)
    raise Refused(code)


def configure():
    for name in ("home", "claude", "codex", "work", "run", "tmux", "state"):
        (ROOT / name).mkdir(mode=0o700)
    os.environ.update({
        "PATH": "/fixture/bin:/usr/bin:/bin",
        "HOME": str(ROOT / "home"),
        "XDG_RUNTIME_DIR": str(ROOT / "run"),
        "TMUX_TMPDIR": str(ROOT / "tmux"),
        "AGENT_SESSION_STATE_DIR": str(ROOT / "state"),
        "CLAUDE_CONFIG_DIR": str(ROOT / "claude"),
        "CODEX_HOME": str(ROOT / "codex"),
        "SHELL": "/bin/sh",
        "LANG": "C.UTF-8",
        "TERM": "xterm-256color",
    })


def versions_match(versions):
    banners = {
        "agent-session": f"agent-session {versions['agent-session']}",
        "agent-hook": f"agent-hook {versions['agent-hook']}",
        "claude": f"{versions['claude']} (Claude Code)",
        "codex": f"codex-cli {versions['codex']}",
    }
    for binary, expected in banners.items():
        result = subprocess.run([binary, "--version"], capture_output=True, text=True, timeout=5)
        banner = result.stdout.strip().split(" (", 1)[0] if binary.startswith("agent-") else result.stdout.strip()
        if result.returncode or banner != expected:
            return False
    return True


def configure_providers(endpoint):
    # Claude's local gateway requires a nonempty token string. This constant is
    # minted by the fixture and can only reach the isolated loopback responder.
    os.environ.update({
        "ANTHROPIC_BASE_URL": endpoint,
        "ANTHROPIC_AUTH_TOKEN": "synthetic-only",
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC": "1",
        "AGENT_SESSION_CODEX_RUNTIME": "app-server",
        "AGENT_SESSION_BIN": "/fixture/bin/agent-session",
    })
    workdir = str(ROOT / "work")
    onboarding = {
        "hasCompletedOnboarding": True,
        "projects": {workdir: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True}},
    }
    for directory in ("home", "claude"):
        (ROOT / directory / ".claude.json").write_text(json.dumps(onboarding), encoding="utf-8")

    (ROOT / "codex/config.toml").write_text(f'''
model = "gpt-5.4"
model_provider = "fixture"
[model_providers.fixture]
name = "Fixture"
base_url = "{endpoint}/v1"
wire_api = "responses"
[projects."{workdir}"]
trust_level = "trusted"
[tui]
show_tooltips = false
''', encoding="utf-8")

    # Use real provider hooks for turn-state admission. The profile never
    # writes activity snapshots, transcripts or native notice receipts.
    policy = b'''schema_version = "agent-hook.policy.v1"
bundle_id = "native-notice-fixture"
version = "1.0.0"
[[rules]]
id = "fixture.claude-activity"
products = ["claude"]
events = ["SessionStart", "UserPromptSubmit", "Stop", "Notification"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
'''
    (ROOT / "policy.toml").write_bytes(policy)
    config = ROOT / "home/.config/agent-hook"
    config.mkdir(parents=True)
    (config / "config.toml").write_text(f'''schema_version = "agent-hook.config.v1"
[policy]
path = "{ROOT / 'policy.toml'}"
digest = "sha256:{hashlib.sha256(policy).hexdigest()}"
''', encoding="utf-8")
    run_cli("activity", "setup", "--agent", "claude", "--apply")


def find_session(session_id):
    return next(
        (row for row in request("/sessions")["sessions"] if row["id"] == session_id),
        None,
    )


def ready_history(agent, session_id):
    view = find_session(session_id)
    if not view or view.get("turn_state", {}).get("phase") != "waiting":
        return None
    provider_id = view.get("provider_resume", {}).get("session_id")
    if not provider_id or not view.get("session_incarnation"):
        return None
    page = request(f"/history/sessions?provider={agent}&q={quote(provider_id, safe='')}")
    match = next(
        (row for row in page["sessions"] if row["provider_session_id"] == provider_id),
        None,
    )
    if match and any(row["text"] == SEED and row["role"] == "user" for row in messages(match["id"])):
        return view, match["id"]
    return None


def messages(history_id):
    page = request(f"/history/sessions/{quote(history_id, safe='')}/messages?direction=forward&limit=50")
    if page.get("next_cursor"):
        raise Refused("history-page-exceeds-fixture-bound")
    return page["messages"]


def observe_rows(history_id):
    rows = messages(history_id)
    if not any(row["text"] == LOOKALIKE for row in rows):
        return None
    notices = [row for row in rows if row["text"] == NOTICE]
    lookalikes = [row for row in rows if row["text"] == LOOKALIKE]
    if (
        len(notices) != 1 or len(lookalikes) != 1
        or sum(row["role"] == "notice" for row in rows) != 1
        or notices[0]["role"] != "notice" or notices[0].get("row_class") != "notice"
        or lookalikes[0]["role"] != "user" or lookalikes[0].get("row_class") is not None
        or rows.index(notices[0]) >= rows.index(lookalikes[0])
    ):
        raise Refused("history-classification-mismatch")
    return {
        "notice": {"count": 1, "role": "notice", "row_class": "notice"},
        "lookalike": {"count": 1, "role": "user", "row_class": None},
    }


def exercise(agent):
    session_id = f"{agent}-fixture"
    view, history_id = wait_for(
        lambda: ready_history(agent, session_id), 18, "provider-not-ready",
    )
    fences = (
        "--expect-runtime", view["session_incarnation"],
        "--expect-provider-session", view["provider_resume"]["session_id"],
    )
    receipt = run_cli(
        "notice", session_id, *NOTICE_ARGS, *fences, "--idempotency-key", "fixture-native-once",
        text=NOTICE_BODY,
    )
    transport = receipt.get("native", {}).get("transport")
    if transport != AGENTS[agent] or receipt.get("submission", {}).get("enter_presses") != 0:
        raise Refused("native-notice-unverified")

    # Wait for the actual provider turn to finish before using ordinary TUI
    # input. This sends a user row without the notice ingress or its claim.
    def notice_settled():
        snapshot = find_session(session_id)
        if not snapshot or snapshot.get("turn_state", {}).get("phase") != "waiting":
            return None
        rows = messages(history_id)
        for index, row in enumerate(rows[:-1]):
            if row["text"] == NOTICE and rows[index + 1]["text"] == inference.REPLY:
                return True
        return None

    wait_for(notice_settled, 12, "provider-not-ready-after-notice")
    run_cli("send", session_id, "--text-stdin", "--key", "enter", text=LOOKALIKE)
    try:
        proof = wait_for(lambda: observe_rows(history_id), 12, "history-lookalike-missing")
    except Refused as err:
        return {"status": "FAIL", "code": err.code}
    return {"status": "PASS", "transport": transport, **proof}


def cleanup(created):
    deleted = 0
    for session_id in created:
        try:
            # Ordinary Linux deletion requires a dedicated systemd pane scope.
            # Here agent-session verifies the exact tmux target and deletes its
            # record, and bubblewrap reaps any remaining namespace processes.
            run_cli("delete", session_id, "--leave-processes")
            deleted += 1
        except Refused:
            continue
    current = run_cli("list")
    tmux_socket = ROOT / "tmux" / f"tmux-{os.getuid()}" / "default"
    if tmux_socket.exists():
        subprocess.run(
            ["tmux", "-S", str(tmux_socket), "kill-server"],
            capture_output=True, timeout=5,
        )

    def server_absent():
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(1)
            try:
                client.connect(str(tmux_socket))
            except (ConnectionRefusedError, FileNotFoundError):
                return True
        return None

    try:
        tmux_absent = wait_for(server_absent, 4, "tmux-server-still-present")
    except Refused:
        tmux_absent = False
    return {"deleted_sessions": deleted, "remaining_sessions": len(current),
            "tmux_server_absent": tmux_absent}


def run_enabled():
    os.environ["AGENT_SESSION_NOTICE_WRITERS"] = "1"
    endpoint = inference.start()
    with endpoint:
        configure_providers(f"http://127.0.0.1:{endpoint.server_port}")
        daemon = subprocess.Popen(
            ["agent-session", "serve", "--bind", CONTROL.removeprefix("http://"), "--token-stdin"],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        created = []
        harnesses = {}
        try:
            daemon.stdin.write(f"{TOKEN}\n".encode())
            daemon.stdin.close()
            wait_for(serve_ready, 8, "serve-startup-unavailable")
            for agent in AGENTS:
                session_id = f"{agent}-fixture"
                created.append(session_id)  # A refused or timed-out create can retain a record.
                try:
                    request("/sessions", {
                        "agent": agent, "id": session_id, "cwd": str(ROOT / "work"), "prompt": SEED,
                        "agent_args": ["--model", "claude-sonnet-4-6"] if agent == "claude" else [],
                    }, timeout=35)
                except Refused as err:
                    harnesses[agent] = unrun(err.code)
            for agent in AGENTS:
                if agent not in harnesses:
                    try:
                        harnesses[agent] = exercise(agent)
                    except Refused as err:
                        harnesses[agent] = unrun(err.code)
        finally:
            try:
                evidence = cleanup(created)
            finally:
                daemon.terminate()
                try:
                    daemon.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
        good = all(row["status"] == "PASS" for row in harnesses.values())
        clean = (
            evidence["tmux_server_absent"] and evidence["remaining_sessions"] == 0
            and (not good or evidence["deleted_sessions"] == 2)
        )
        status, exit_code = ("PASS", 0) if good else ("UNRUN", 2)
        if not clean or any(row["status"] == "FAIL" for row in harnesses.values()):
            status, exit_code = "FAIL", 1
        return {"status": status, "harnesses": harnesses, "cleanup": evidence}, exit_code


def serve_ready():
    try:
        request("/sessions")
        return True
    except Refused as err:
        if err.code != "control-unavailable":
            raise
        return None


def main(versions, enabled):
    # Use an in-namespace process session so the installed CLI can verify
    # pane process/session identity through the fixture's procfs.
    os.setsid()
    os.umask(0o077)
    configure()
    if not versions_match(versions):
        return {"status": "UNRUN", "created": 0, "harnesses": {
            agent: unrun("fixture-version-unsupported") for agent in AGENTS
        }}, 2
    if enabled:
        return run_enabled()

    harnesses = {}
    for agent in AGENTS:
        try:
            run_cli("notice", f"{agent}-fixture", *NOTICE_ARGS, text=NOTICE_BODY)
        except Refused as err:
            harnesses[agent] = unrun(err.code)
    gate_refused = all(harnesses.get(agent) == unrun("notice-writers-disabled") for agent in AGENTS)
    return {
        "status": "UNRUN" if gate_refused else "FAIL",
        "harnesses": harnesses,
        "created": 0,
    }, 2 if gate_refused else 1


if __name__ == "__main__":
    try:
        result, code = main(json.loads(sys.argv[1]), sys.argv[2] == "enabled")
    except Refused as error:
        result, code = {"status": "FAIL", "code": error.code}, 1
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired):
        result, code = {"status": "FAIL", "code": "fixture-runtime-error"}, 1
    print(json.dumps(result), flush=True)
    sys.exit(code)
