"""Synthetic CLI acceptance tests for the read-only retirement inventory."""

import datetime
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


PROBE = Path(__file__).with_name("closed-ledger-retirement-probe.py")
NOW = 1_800_000_000
WEEK = 7 * 24 * 60 * 60


def entry(seq, age=0, **fields):
    closed = datetime.datetime.fromtimestamp(NOW - age, datetime.timezone.utc)
    return {
        "seq": seq,
        "record": {"closed_at": closed.isoformat(), **fields},
    }


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.ledger = Path(self.tmp.name) / "closed-ledger.json"

    def write(self, entries):
        self.ledger.write_text(json.dumps({
            "schema_version": "agent-session.board-closed-ledger.v1",
            "ledger_id": "00000000-0000-0000-0000-000000000001",
            "last_seq": entries[-1]["seq"] if entries else 0,
            "entries": entries,
        }))
        self.ledger.chmod(0o600)

    def probe(self, expected_exit=0):
        result = subprocess.run(
            [sys.executable, str(PROBE), "--ledger", str(self.ledger),
             "--now-epoch", str(NOW)],
            capture_output=True, text=True, timeout=5, check=False,
        )
        self.assertEqual(result.returncode, expected_exit, result.stderr)
        self.assertEqual(result.stderr, "")
        self.assertNotIn(str(self.ledger), result.stdout)
        self.assertNotIn("sensitive-fixture-value", result.stdout)
        return json.loads(result.stdout)

    def test_counts_visible_old_shapes_without_values_writes_or_writer_lock(self):
        rows = [
            entry(1, launch_env={"sensitive-fixture-value": "hidden"}),
            entry(2),
            entry(3, WEEK),
            entry(4, WEEK + 1),
            {"seq": 5, "record": {"closed_at": "unreadable"}},
            entry(6, launch_env=None),
            entry(7, -1, launch_env={}),
        ]
        self.write(rows)
        before = self.ledger.read_bytes(), self.ledger.stat()
        # The live writer locks a separate file. The probe must not wait on it.
        with self.ledger.with_suffix(".lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            report = self.probe()
        self.assertEqual(report, {
            "schema_version": "agent-session.closed-ledger-retirement-probe.v1",
            "observed_at_epoch": NOW,
            "status": "observed",
            "counts": {
                "stored_entries": 7,
                "age_pruned_entries": 2,
                "count_pruned_entries": 0,
                "unreadable_closed_at_entries": 1,
                "visible_entries": 5,
                "visible_missing_launch_env": 2,
                "visible_invalid_launch_env": 1,
                "visible_launch_env_objects": 2,
                "visible_future_closed_at": 1,
            },
        })
        after = self.ledger.read_bytes(), self.ledger.stat()
        self.assertEqual(before[0], after[0])
        for field in ("st_ino", "st_size", "st_mtime_ns", "st_ctime_ns", "st_mode"):
            self.assertEqual(getattr(before[1], field), getattr(after[1], field))

    def test_count_limit_is_applied_after_age_pruning_in_sequence_order(self):
        rows = [entry(1), entry(2, WEEK + 1)]
        rows.extend(entry(seq, launch_env={}) for seq in range(3, 260))
        self.write(rows)
        counts = self.probe()["counts"]
        self.assertEqual(counts["age_pruned_entries"], 1)
        self.assertEqual(counts["count_pruned_entries"], 2)
        self.assertEqual(counts["visible_entries"], 256)
        self.assertEqual(counts["visible_missing_launch_env"], 0)

    def test_missing_is_unobserved_and_does_not_create_state(self):
        report = self.probe()
        self.assertEqual(report["status"], "absent")
        self.assertIsNone(report["counts"])
        self.assertFalse(self.ledger.exists())
        self.assertEqual(list(Path(self.tmp.name).iterdir()), [])

    def test_empty_current_ledger_is_an_observed_zero(self):
        self.write([])
        report = self.probe()
        self.assertEqual(report["status"], "observed")
        self.assertTrue(all(count == 0 for count in report["counts"].values()))

    def test_timestamp_offsets_fractional_seconds_and_unreadable_dates(self):
        rows = [entry(seq) for seq in range(1, 5)]
        # Both valid stamps identify the exact inclusive seven-day boundary.
        boundary = datetime.datetime.fromtimestamp(NOW - WEEK, datetime.timezone.utc)
        offset = datetime.timezone(datetime.timedelta(hours=2))
        rows[0]["record"]["closed_at"] = boundary.astimezone(offset).isoformat()
        rows[1]["record"]["closed_at"] = boundary.strftime("%Y-%m-%dT%H:%M:%S.999999999Z")
        rows[2]["record"]["closed_at"] = "2027-01-01 00:00:00"
        rows[3]["record"]["closed_at"] = "2027-02-30T00:00:00Z"
        self.write(rows)
        counts = self.probe()["counts"]
        self.assertEqual(counts["visible_missing_launch_env"], 2)
        self.assertEqual(counts["unreadable_closed_at_entries"], 2)

    def test_invalid_ledgers_never_become_zero_or_disclose_content(self):
        for content in (
            "sensitive-fixture-value",
            json.dumps({"schema_version": "retired", "secret": "sensitive-fixture-value"}),
            json.dumps({"schema_version": "agent-session.board-closed-ledger.v1",
                        "ledger_id": "invalid", "last_seq": 0, "entries": []}),
        ):
            with self.subTest(content=content[:20]):
                self.ledger.write_text(content)
                self.ledger.chmod(0o600)
                report = self.probe(expected_exit=2)
                self.assertEqual(report["status"], "invalid")
                self.assertIsNone(report["counts"])
                self.assertEqual(self.ledger.read_text(), content)

    def test_out_of_order_or_out_of_range_sequence_is_invalid(self):
        for rows in ([entry(2), entry(1)], [entry(0)], [entry(1), entry(1)]):
            with self.subTest(rows=rows):
                self.write(rows)
                self.assertEqual(self.probe(2)["status"], "invalid")

    def test_untrusted_file_is_not_read_and_symlink_is_not_followed(self):
        self.write([entry(1)])
        self.ledger.chmod(0o644)
        self.assertEqual(self.probe(2)["status"], "unavailable")
        self.ledger.chmod(0o600)
        original = self.ledger.with_suffix(".original")
        self.ledger.rename(original)
        self.ledger.symlink_to(original)
        self.assertEqual(self.probe(2)["status"], "unavailable")

    def test_non_regular_input_does_not_block(self):
        os.mkfifo(self.ledger, 0o600)
        self.assertEqual(self.probe(2)["status"], "unavailable")


if __name__ == "__main__":
    unittest.main()
