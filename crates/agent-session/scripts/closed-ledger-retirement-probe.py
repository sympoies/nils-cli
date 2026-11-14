#!/usr/bin/env python3
"""Count retained closed-ledger shapes without projecting or changing state.

This is an operator-run #3081 inventory, not a migration or a fleet receipt.
The visibility calculation follows board/ledger.rs::Ledger::prune. Opening an
existing inode without a lock gives one snapshot of the atomically saved ledger.
Only fixed statuses and aggregate counts are emitted, never record values.
"""

import argparse
import datetime
import json
import os
import re
import stat
import time
import uuid


MAX_BYTES = 16 * 1024 * 1024
MAX_AGE_SECONDS = 7 * 24 * 60 * 60
MAX_ENTRIES = 256
LEDGER_SCHEMA = "agent-session.board-closed-ledger.v1"
REPORT_SCHEMA = "agent-session.closed-ledger-retirement-probe.v1"
TIMESTAMP = re.compile(
    r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]{1,9})?(?:Z|[+-][0-9]{2}:[0-9]{2})"
)


def integer(value):
    return isinstance(value, int) and not isinstance(value, bool)


def closed_at_epoch(record):
    raw = record.get("closed_at") if isinstance(record, dict) else None
    if not isinstance(raw, str) or TIMESTAMP.fullmatch(raw) is None:
        return None
    try:
        timestamp = datetime.datetime.fromisoformat(raw.replace("Z", "+00:00"))
        if timestamp.tzinfo is None:
            return None
        return int(timestamp.timestamp())
    except (ValueError, OverflowError):
        return None


def valid_ledger(ledger):
    if not isinstance(ledger, dict) or set(ledger) != {
        "schema_version", "ledger_id", "last_seq", "entries"
    }:
        return False
    if ledger["schema_version"] != LEDGER_SCHEMA:
        return False
    try:
        uuid.UUID(ledger["ledger_id"])
    except (ValueError, TypeError, AttributeError):
        return False
    last = ledger["last_seq"]
    if not integer(last) or not 0 <= last <= 2**64 - 1:
        return False
    if not isinstance(ledger["entries"], list):
        return False
    previous = 0
    for entry in ledger["entries"]:
        if not isinstance(entry, dict) or set(entry) != {"seq", "record"}:
            return False
        seq = entry["seq"]
        if not integer(seq) or not previous < seq <= last:
            return False
        previous = seq
    return True


def counts(ledger, now):
    entries = ledger["entries"]
    visible = []
    unreadable = 0
    for entry in entries:
        closed = closed_at_epoch(entry["record"])
        if closed is None:
            unreadable += 1
        elif now - closed <= MAX_AGE_SECONDS:
            visible.append((entry["record"], closed))
    age_pruned = len(entries) - len(visible)
    count_pruned = max(0, len(visible) - MAX_ENTRIES)
    visible = visible[-MAX_ENTRIES:]
    result = {
        "stored_entries": len(entries),
        "age_pruned_entries": age_pruned,
        "count_pruned_entries": count_pruned,
        "unreadable_closed_at_entries": unreadable,
        "visible_entries": len(visible),
        "visible_missing_launch_env": 0,
        "visible_invalid_launch_env": 0,
        "visible_launch_env_objects": 0,
        "visible_future_closed_at": 0,
    }
    for record, closed in visible:
        result["visible_future_closed_at"] += closed > now
        if "launch_env" not in record:
            result["visible_missing_launch_env"] += 1
        elif not isinstance(record["launch_env"], dict):
            result["visible_invalid_launch_env"] += 1
        else:
            result["visible_launch_env_objects"] += 1
    return result


def inspect(path, now):
    try:
        descriptor = os.open(
            path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
        )
    except FileNotFoundError:
        return "absent", None
    except OSError:
        return "unavailable", None
    try:
        with os.fdopen(descriptor, "rb") as source:
            metadata = os.fstat(source.fileno())
            if (not stat.S_ISREG(metadata.st_mode)
                    or metadata.st_uid != os.geteuid()
                    or metadata.st_mode & 0o077
                    or metadata.st_nlink != 1):
                return "unavailable", None
            raw = source.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            return "invalid", None
        ledger = json.loads(raw)
        if not valid_ledger(ledger):
            return "invalid", None
        return "observed", counts(ledger, now)
    except (OSError, ValueError, RecursionError):
        return "invalid", None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", required=True, help="exact closed-ledger.json path")
    parser.add_argument("--now-epoch", type=int, help="observation epoch; defaults to now")
    args = parser.parse_args()
    now = int(time.time()) if args.now_epoch is None else args.now_epoch
    status, observed_counts = inspect(args.ledger, now)
    print(json.dumps({
        "schema_version": REPORT_SCHEMA,
        "observed_at_epoch": now,
        "status": status,
        "counts": observed_counts,
    }, sort_keys=True))
    return 0 if status in {"observed", "absent"} else 2


if __name__ == "__main__":
    raise SystemExit(main())
