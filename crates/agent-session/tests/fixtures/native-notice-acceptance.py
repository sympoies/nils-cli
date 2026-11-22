"""Run the installed native notice acceptance profile in a throwaway Linux sandbox."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


VERSIONS = {"agent-session": "1.42.0", "agent-hook": "1.42.0",
            "claude": "2.1.317", "codex": "0.162.1"}
RESOURCES = {
    "state_roots": 1, "provider_homes": 2, "tmux_servers": 1,
    "serve_daemons": 1, "inference_servers": 1, "sessions": 2,
    "native_notices": 2, "typed_lookalikes": 2,
}
SCHEMA = "agent-session.native-notice-acceptance.v1"


def refusal(code):
    return {
        "status": "UNRUN",
        "harnesses": {
            agent: {"status": "UNRUN", "code": code} for agent in ("claude", "codex")
        },
        "created": 0,
    }


def executable(value):
    path = Path(value).resolve(strict=True)
    # Mount only native binaries. A package launcher can depend on an entire
    # ambient installation and would make the sandbox's inputs ambiguous.
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError("expected an executable file")
    with path.open("rb") as binary:
        if binary.read(4) != b"\x7fELF":
            raise ValueError("expected a native executable")
    return path


def run(args):
    bubblewrap = shutil.which("bwrap")
    if not sys.platform.startswith("linux") or not bubblewrap:
        return refusal("sandbox-unavailable"), 2

    try:
        session = executable(args.agent_session)
        binaries = {
            "agent-session": session,
            "agent-hook": executable(session.with_name("agent-hook")),
            "claude": executable(args.claude),
            "codex": executable(args.codex),
        }
    except (OSError, ValueError):
        return refusal("native-binary-unavailable"), 2

    fixture = Path(__file__).resolve().parent
    command = [
        bubblewrap, "--unshare-user", "--unshare-pid", "--unshare-net",
        "--unshare-ipc", "--unshare-uts", "--hostname", "native-notice-fixture",
        "--die-with-parent", "--clearenv", "--dev", "/dev", "--proc", "/proc",
        "--tmpfs", "/tmp", "--tmpfs", "/f",
    ]
    for path in ("/usr", "/bin", "/lib", "/lib64", "/etc/passwd", "/etc/group", "/etc/terminfo"):
        command.extend(("--ro-bind-try", path, path))
    # System tooling is visible, while user homes, managed provider config and
    # host runtime sockets are outside the mount namespace.
    command.extend(("--tmpfs", "/usr/local"))
    for name, binary in binaries.items():
        command.extend(("--ro-bind", str(binary), f"/fixture/bin/{name}"))
    for name in ("native_notice_runtime.py", "native_notice_inference.py"):
        command.extend(("--ro-bind", str(fixture / name), f"/fixture/{name}"))

    # The worker imports only the mounted fixture and the system standard library.
    command.extend((
        "--chdir", "/f", "/usr/bin/python3", "-E", "-s", "-B",
        "/fixture/native_notice_runtime.py", json.dumps(VERSIONS),
        "enabled" if args.enable_writers else "disabled",
    ))
    try:
        process = subprocess.run(command, capture_output=True, text=True, timeout=240)
    except subprocess.TimeoutExpired:
        # Bubblewrap owns the PID namespace and tears it down even on timeout.
        return {"status": "FAIL", "code": "fixture-timeout"}, 1
    except OSError:
        return refusal("sandbox-unavailable"), 2

    try:
        result = json.loads(process.stdout)
    except ValueError:
        return refusal("sandbox-unavailable"), 2
    return result, process.returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="report counts without creating resources")
    parser.add_argument("--enable-writers", action="store_true", help="opt in to notice writers inside the fixture")
    parser.add_argument("--agent-session", help="native agent-session binary with its matching agent-hook sibling")
    parser.add_argument("--claude", help="native Claude Code binary")
    parser.add_argument("--codex", help="native Codex CLI binary")
    args = parser.parse_args()

    if args.dry_run:
        result, exit_code = {"status": "DRY_RUN", "resources": RESOURCES, "created": 0}, 0
    else:
        if not all((args.agent_session, args.claude, args.codex)):
            parser.error("provide --agent-session, --claude, and --codex for a real run")
        result, exit_code = run(args)
    print(json.dumps({"schema_version": SCHEMA, "versions": VERSIONS, **result}, sort_keys=True))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
