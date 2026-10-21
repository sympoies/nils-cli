#!/usr/bin/env python3
"""Exercise the shared-target replacement guard (sympoies/nils-cli#2697)."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[3]
GUARD = REPO / 'scripts/ci/lib/target_guard.py'


def write_exec(path, content, mtime_ns):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    path.chmod(0o755)
    os.utime(path, ns=(mtime_ns, mtime_ns))


class TargetGuardTest(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory(prefix='target-guard-')
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self.target = self.root / 'target'
        self.snapshot_file = self.root / 'snapshot.json'
        self.bin = self.target / 'debug/agent-session'
        self.test_bin = self.target / 'debug/deps/agent_session-0123abcd'
        write_exec(self.bin, 'bin-v1', 1_000_000_000)
        write_exec(self.test_bin, 'test-v1', 1_000_000_000)

    def guard(self, *args):
        return subprocess.run(
            [sys.executable, str(GUARD), *args], text=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)

    def snapshot(self):
        result = self.guard(
            'snapshot', '--target-dir', str(self.target), '--out', str(self.snapshot_file))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_unchanged_target_verifies(self):
        self.snapshot()
        result = self.guard('verify', str(self.snapshot_file))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_replaced_binary_is_reported_as_shared_target_replaced(self):
        self.snapshot()
        write_exec(self.bin, 'bin-from-another-worktree', 2_000_000_000)
        result = self.guard('verify', str(self.snapshot_file))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('shared target replaced', result.stdout)
        self.assertIn('debug/agent-session', result.stdout)

    def test_replaced_test_executable_is_reported(self):
        self.snapshot()
        write_exec(self.test_bin, 'test-from-another-worktree', 2_000_000_000)
        result = self.guard('verify', str(self.snapshot_file))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('shared target replaced', result.stdout)
        self.assertIn('debug/deps/agent_session-0123abcd', result.stdout)

    def test_removed_binary_is_reported(self):
        self.snapshot()
        self.bin.unlink()
        result = self.guard('verify', str(self.snapshot_file))
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('shared target replaced', result.stdout)
        self.assertIn('debug/agent-session', result.stdout)

    def test_non_executable_artifacts_are_ignored(self):
        rlib = self.target / 'debug/deps/libfoo-0123abcd.rlib'
        write_exec(rlib, 'rlib-v1', 1_000_000_000)
        dep_info = self.target / 'debug/deps/agent_session-0123abcd.d'
        dep_info.write_text('deps-v1')
        self.snapshot()
        write_exec(rlib, 'rlib-v2', 2_000_000_000)
        dep_info.write_text('deps-v2')
        self.test_bin.with_name('fresh-0123abcd').write_text('not-executable')
        result = self.guard('verify', str(self.snapshot_file))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_missing_target_dir_snapshots_empty(self):
        result = self.guard(
            'snapshot', '--target-dir', str(self.root / 'absent'),
            '--out', str(self.snapshot_file))
        self.assertEqual(result.returncode, 0, result.stdout)
        result = self.guard('verify', str(self.snapshot_file))
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == '__main__':
    unittest.main()
