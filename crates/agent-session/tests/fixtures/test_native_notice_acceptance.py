"""Contract checks for the installed native notice acceptance profile."""

import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


PROFILE = Path(__file__).with_name("native-notice-acceptance.py")
HARNESS_TRANSPORTS = {"claude": "claude_mod", "codex": "codex_app_server"}
BINARIES = ("AGENT_SESSION_BIN", "CLAUDE_BIN", "CODEX_BIN")


class NativeNoticeAcceptance(unittest.TestCase):
    def invoke(self, *args, expected_exit=0):
        self.assertTrue(PROFILE.is_file(), "installed native notice profile is missing")
        run = subprocess.run(
            [sys.executable, "-I", str(PROFILE), *args],
            capture_output=True, text=True, timeout=250, check=False,
        )
        self.assertEqual(run.returncode, expected_exit, run.stdout + run.stderr)
        self.assertEqual(run.stderr, "")
        return json.loads(run.stdout)

    def test_dry_run_reports_bounded_plan(self):
        plan = self.invoke("--dry-run")
        self.assertEqual(plan["status"], "DRY_RUN")
        self.assertEqual(plan["resources"], {
            "state_roots": 1, "provider_homes": 2, "tmux_servers": 1,
            "serve_daemons": 1, "inference_servers": 1, "sessions": 2,
            "native_notices": 2, "typed_lookalikes": 2,
        })
        self.assertEqual(plan["created"], 0)

    def installed_args(self):
        if not all(os.environ.get(key) for key in BINARIES):
            self.skipTest("set AGENT_SESSION_BIN, CLAUDE_BIN, and CODEX_BIN to run installed acceptance")
        return [
            "--agent-session", os.environ["AGENT_SESSION_BIN"],
            "--claude", os.environ["CLAUDE_BIN"],
            "--codex", os.environ["CODEX_BIN"],
        ]

    def test_installed_native_delivery_and_cleanup(self):
        result = self.invoke("--enable-writers", *self.installed_args())
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(set(result["harnesses"]), set(HARNESS_TRANSPORTS))
        for agent, transport in HARNESS_TRANSPORTS.items():
            with self.subTest(agent=agent):
                self.assertEqual(result["harnesses"][agent], {
                    "status": "PASS",
                    "transport": transport,
                    "notice": {"count": 1, "role": "notice", "row_class": "notice"},
                    "lookalike": {"count": 1, "role": "user", "row_class": None},
                })
        self.assertEqual(result["cleanup"], {
            "deleted_sessions": 2, "remaining_sessions": 0, "tmux_server_absent": True,
        })

    def test_gate_off_refuses_without_creating_sessions(self):
        result = self.invoke(*self.installed_args(), expected_exit=2)
        self.assertEqual(result["status"], "UNRUN")
        self.assertEqual(result["harnesses"], {
            agent: {"status": "UNRUN", "code": "notice-writers-disabled"}
            for agent in HARNESS_TRANSPORTS
        })
        self.assertEqual(result["created"], 0)


    def test_unsupported_pin_refuses_without_creating_sessions(self):
        args = self.installed_args()
        args[args.index("--codex") + 1] = os.environ["CLAUDE_BIN"]
        result = self.invoke("--enable-writers", *args, expected_exit=2)
        self.assertEqual(result["status"], "UNRUN")
        self.assertEqual(result["harnesses"], {
            agent: {"status": "UNRUN", "code": "fixture-version-unsupported"}
            for agent in HARNESS_TRANSPORTS
        })
        self.assertEqual(result["created"], 0)

if __name__ == "__main__":
    unittest.main()
