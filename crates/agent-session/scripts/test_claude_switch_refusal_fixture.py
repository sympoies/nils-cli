"""Contract tests against a caller-selected agent-session executable."""

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

FIXTURE = Path(__file__).with_name("claude-switch-refusal-fixture.py")
RUNTIME = os.environ.get("CLAUDE_SWITCH_TEST_AGENT_SESSION")
REFUSAL = {
    "http_status": 422,
    "code": "claude-account-switch-refused",
    "cause": "claude-account-broker-rejected",
}


class RefusalFixtureTests(unittest.TestCase):
    def run_fixture(self, *args, exit_code=0):
        self.assertTrue(FIXTURE.is_file(), "the installed-runtime fixture entry point is missing")
        run = subprocess.run(
            [sys.executable, "-I", str(FIXTURE), *args],
            capture_output=True,
            text=True,
            timeout=130,
            env={
                **os.environ,
                "AGENT_SESSION_CLAUDE_BIN": "/nonexistent/inherited-claude",
                "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER": '["/nonexistent/inherited-broker"]',
            },
            check=False,
        )
        self.assertEqual(run.returncode, exit_code, run.stdout + run.stderr)
        self.assertEqual(run.stderr, "")
        return json.loads(run.stdout)

    def test_dry_run_counts_and_missing_runtime(self):
        planned = self.run_fixture("--agent-session", "/nonexistent/agent-session", "--dry-run")
        self.assertEqual(planned["status"], "DRY_RUN")
        self.assertEqual(planned["counts"], {
            "private_roots": 1,
            "serve_daemons": 1,
            "tmux_servers": 1,
            "managed_sessions": 1,
            "synthetic_provider_processes": 1,
            "account_nicknames": 2,
            "materialized_account_stores": 1,
        })
        unrun = self.run_fixture("--agent-session", "/nonexistent/agent-session", exit_code=2)
        self.assertEqual(unrun["status"], "UNRUN")
        self.assertEqual(unrun["reason"], "agent-session-unavailable")

    @unittest.skipUnless(RUNTIME, "set CLAUDE_SWITCH_TEST_AGENT_SESSION to an installed or built binary")
    def test_real_route_is_repeatably_typed_without_mutation(self):
        consumer = """
import http.client, json, os, pathlib, sys, urllib.parse
env = os.environ
url = urllib.parse.urlsplit(env["AGENT_SESSION_FIXTURE_URL"])
token_file = pathlib.Path(env["AGENT_SESSION_FIXTURE_TOKEN_FILE"])
assert token_file.stat().st_mode & 0o777 == 0o600
pathlib.Path(sys.argv[1]).write_text(str(token_file.parent))
connection = http.client.HTTPConnection(url.hostname, url.port, timeout=15)
path = "/sessions/" + env["AGENT_SESSION_FIXTURE_SESSION_ID"] + "/account"
body = json.dumps({"account": env["AGENT_SESSION_FIXTURE_TARGET_ACCOUNT"],
                   "expected_session_incarnation": env["AGENT_SESSION_FIXTURE_INCARNATION"]})
connection.request("PUT", path, body, {"Content-Type": "application/json"})
response = connection.getresponse()
assert response.status == 401
response.read()
connection.request("PUT", path, body, {"Content-Type": "application/json",
    "Authorization": "Bearer " + token_file.read_text().strip()})
response = connection.getresponse()
error = json.load(response)["error"]
assert response.status == 422
assert error["code"] == "claude-account-switch-refused"
assert error["details"]["cause"] == "claude-account-broker-rejected"
"""
        with tempfile.TemporaryDirectory() as destination:
            root_record = Path(destination) / "fixture-root"
            report = self.run_fixture(
                "--agent-session", RUNTIME, "--", sys.executable, "-I", "-c", consumer,
                str(root_record),
            )
            root = Path(root_record.read_text())
            self.assertFalse(root.exists())
            self.assertNotIn(str(root), json.dumps(report))

        self.assertEqual(report["status"], "PASS")
        self.assertEqual(report["refusal"], REFUSAL)
        self.assertEqual(report["target_materialization_attempts"], 2)
        self.assertEqual(report["before"], report["after"])
        before = report["before"]
        self.assertEqual(before["status"], "running")
        self.assertEqual(before["turn_phase"], "waiting")
        self.assertEqual(before["account"]["selected"], "fixture-source")
        self.assertEqual(before["account"]["revision"], 1)
        self.assertIsNone(before["account"]["next"])
        self.assertTrue(before["provider_session_id"])
        self.assertTrue(before["provider_pid"])
        self.assertEqual(before["provider_launches"], 1)
        self.assertTrue(report["account_store_unchanged"])
        self.assertEqual(report["cleanup"], {
            "daemon_absent": True,
            "session_absent": True,
            "tmux_absent": True,
            "provider_absent": True,
            "root_absent": True,
        })

    @unittest.skipUnless(RUNTIME, "set CLAUDE_SWITCH_TEST_AGENT_SESSION to an installed or built binary")
    def test_failed_consumer_still_deletes_the_owned_session(self):
        report = self.run_fixture(
            "--agent-session", RUNTIME, "--", sys.executable, "-I", "-c", "raise SystemExit(7)",
            exit_code=1,
        )
        self.assertEqual(report["status"], "FAIL")
        self.assertEqual(report["reason"], "consumer-failed")
        self.assertTrue(report["cleanup"]["session_absent"])
        self.assertTrue(report["cleanup"]["provider_absent"])
        self.assertTrue(report["cleanup"]["root_absent"])


if __name__ == "__main__":
    unittest.main()
