#!/usr/bin/env python3
"""Read-only aggregate retirement inventory for one explicitly supplied state root.

No runtime loaders, repairs, locks, network calls or record values in output.
Each file is an inode snapshot; stores are not a transaction across files.
"""

import argparse
import contextlib
import json
import math
import os
import stat
import sys
import time

# Imports must not create __pycache__ beside the operator's probe.
sys.dont_write_bytecode = True
from retirement_shapes import (  # noqa: E402
    add_auto, add_session, auto_counts, heartbeat_counts, journal_counts,
    registry_counts, session_counts,
)

MAX_BYTES = 64 * 1024 * 1024
MAX_SESSIONS = 10000
FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
STORES = ("sessions", "auto_resume", "registry", "heartbeat", "journal")


class Unobserved(Exception):
    pass


class Invalid(Exception):
    pass


@contextlib.contextmanager
def directory(path, parent=None):
    try:
        descriptor = os.open(path, FLAGS | os.O_DIRECTORY, dir_fd=parent)
    except OSError:
        raise Unobserved() from None
    try:
        if os.fstat(descriptor).st_uid != os.geteuid():
            raise Unobserved()
        yield descriptor
    finally:
        os.close(descriptor)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError()
        result[key] = value
    return result


def reject_constant(_value):
    raise ValueError()


def finite_float(value):
    number = float(value)
    if not math.isfinite(number):
        raise ValueError()
    return number


def read_json(parent, name):
    try:
        descriptor = os.open(name, FLAGS, dir_fd=parent)
    except FileNotFoundError:
        return None
    except OSError:
        raise Unobserved() from None
    try:
        with os.fdopen(descriptor, "rb") as source:
            info = os.fstat(source.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                    or info.st_mode & 0o077 or info.st_nlink != 1):
                raise Unobserved()
            if info.st_size > MAX_BYTES:
                raise Invalid()
            raw = source.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            raise Invalid()
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
                           parse_constant=reject_constant, parse_float=finite_float)
        # Reject lone surrogate escapes as well as non-UTF-8 bytes.
        json.dumps(value, ensure_ascii=False).encode("utf-8")
        if not isinstance(value, dict):
            raise Invalid()
        return value
    except OSError:
        raise Unobserved() from None
    except (ValueError, RecursionError):
        raise Invalid() from None


def observed(counts):
    return {"status": "observed", "counts": counts}


def unavailable(status="unobserved"):
    return {"status": status, "counts": None}


def scan_sessions(root):
    records, resumes = session_counts(), auto_counts()
    resume_failure = None
    try:
        with directory("sessions", root) as sessions:
            # scandir streams the cap; do not allocate an unbounded name list.
            with os.scandir(sessions) as entries:
                for number, entry in enumerate(entries, 1):
                    if number > MAX_SESSIONS:
                        raise Invalid()
                    # Runtime sessions are one directory per record. Unexpected
                    # entries prevent an absence claim; never follow symlinks.
                    with directory(entry.name, sessions) as session:
                        record = read_json(session, "session.json")
                        if record is None:
                            raise Unobserved()
                        add_session(records, record)
                        try:
                            state = read_json(session, "auto-resume.json")
                            if state is None:
                                resumes["absent_files"] += 1
                            else:
                                add_auto(resumes, state, record["agent"])
                        except Unobserved:
                            resume_failure = resume_failure or "unobserved"
                        except Invalid:
                            # Invalid takes priority over an unreadable sidecar.
                            resume_failure = "invalid"
    except (Invalid, ValueError, RecursionError):
        return unavailable("invalid"), unavailable("invalid")
    except (Unobserved, OSError):
        return unavailable(), unavailable()
    return observed(records), unavailable(resume_failure) if resume_failure else observed(resumes)


def scan_store(root, filename, counter):
    try:
        with directory("coordination", root) as coordination:
            value = read_json(coordination, filename)
        return unavailable() if value is None else observed(counter(value))
    except (Invalid, ValueError, RecursionError):
        return unavailable("invalid")
    except (Unobserved, OSError):
        return unavailable()


def inspect(path):
    stores = dict.fromkeys(STORES)
    try:
        with directory(path) as root:
            stores["sessions"], stores["auto_resume"] = scan_sessions(root)
            for key, filename, counter in (
                ("registry", "registry.json", registry_counts),
                ("heartbeat", "broker-state.json", heartbeat_counts),
                ("journal", "federation-journal.json", journal_counts),
            ):
                stores[key] = scan_store(root, filename, counter)
    except Unobserved:
        return "unobserved", {key: unavailable() for key in STORES}
    return "observed", stores


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-root", required=True, help="exact configured state directory")
    parser.add_argument("--now-epoch", type=int, help="observation epoch; defaults to now")
    args = parser.parse_args()
    status, stores = inspect(args.state_root)
    print(json.dumps({
        "schema_version": "agent-session.state-retirement-probe.v1",
        "observed_at_epoch": int(time.time()) if args.now_epoch is None else args.now_epoch,
        "status": status,
        "stores": stores,
        "client_contracts": {"status": "unobserved", "counts": None},
    }, sort_keys=True))
    return 0 if status == "observed" and all(
        store["status"] != "invalid" for store in stores.values()
    ) else 2


if __name__ == "__main__":
    raise SystemExit(main())
