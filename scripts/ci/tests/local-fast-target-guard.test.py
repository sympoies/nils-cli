#!/usr/bin/env python3
"""A binary replaced mid-run fails local-fast as a replaced target, not a product test.

sympoies/nils-cli#2697: lanes that share one cargo target directory can overwrite
the executables a running gate is testing. Mocked Cargo work stands in for the
other lane.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[3]
LOCAL_FAST = REPO / 'scripts/ci/nils-cli-local-fast.sh'

FAKE_CARGO = '''#!{python}
import json, os, subprocess, sys
args = sys.argv[1:]
if args[:1] == ["metadata"]:
    metadata = subprocess.check_output([
        os.environ["LOCAL_FAST_TEST_REAL_CARGO"], "metadata",
        "--no-deps", "--locked", "--all-features",
        "--format-version", "1"], text=True)
    result = json.loads(metadata)
    result["resolve"] = {{"nodes": []}}
    print(json.dumps(result))
    raise SystemExit(0)
with open(os.environ["LOCAL_FAST_TEST_CARGO_LOG"], "a") as out:
    out.write(json.dumps(args) + "\\n")
debug = os.path.join(os.environ["CARGO_TARGET_DIR"], "debug")
os.makedirs(os.path.join(debug, "deps"), exist_ok=True)
def write(path, content, mtime):
    with open(path, "w") as handle:
        handle.write(content)
    os.chmod(path, 0o755)
    os.utime(path, ns=(mtime, mtime))
if args[:1] not in (["build"], ["test"]):
    raise SystemExit(0)
if "--no-run" in args or args[:1] == ["build"]:
    write(os.path.join(debug, "agent-session"), "built-from-gate-head", 1_000_000_000)
    write(os.path.join(debug, "deps", "agent_session-0123abcd"), "test-from-gate-head", 1_000_000_000)
    raise SystemExit(0)
# The test run itself: another lane rebuilds the shared binary mid-run.
if os.environ.get("LOCAL_FAST_TEST_REPLACE") == "1":
    write(os.path.join(debug, "agent-session"), "built-from-another-worktree", 2_000_000_000)
'''


class LocalFastTargetGuardTest(unittest.TestCase):
    def setUp(self):
        self.real_cargo = shutil.which('cargo')
        self.real_bash = shutil.which('bash')
        if not self.real_cargo or not self.real_bash:
            self.skipTest('requires cargo and bash')
        self._temp = tempfile.TemporaryDirectory(prefix='local-fast-target-guard-')
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        bin_dir = self.root / 'bin'
        bin_dir.mkdir()
        self.log = self.root / 'cargo-commands.jsonl'
        cargo = bin_dir / 'cargo'
        cargo.write_text(FAKE_CARGO.format(python=sys.executable))
        cargo.chmod(0o755)
        # Audit scripts are out of scope here; only Cargo work is observed.
        fake_bash = bin_dir / 'bash'
        fake_bash.write_text('#!/bin/sh\nexit 0\n')
        fake_bash.chmod(0o755)
        # Other python3 callers (script test suites) are stubbed; the plan
        # (`-`) and the target guard run for real.
        fake_python = bin_dir / 'python3'
        fake_python.write_text(
            '#!/bin/sh\n'
            'case "${1-}" in\n'
            f'  -|*/target_guard.py) exec {sys.executable!s} "$@" ;;\n'
            'esac\n'
            'exit 0\n'
        )
        fake_python.chmod(0o755)
        cargo_home = self.root / 'cargo-home'
        cargo_home.mkdir()
        self.env = os.environ.copy()
        self.env.update({
            'PATH': f'{bin_dir}{os.pathsep}{self.env.get("PATH", "")}',
            'CARGO_HOME': str(cargo_home),
            'CARGO_TARGET_DIR': str(self.root / 'shared-target'),
            'LOCAL_FAST_TEST_REAL_CARGO': self.real_cargo,
            'LOCAL_FAST_TEST_CARGO_LOG': str(self.log),
            'NILS_CLI_GATE_ACTIVE': '1',
            'NILS_CLI_TEST_RUNNER': 'cargo',
        })

    def run_local_fast(self, **extra_env):
        env = {**self.env, **extra_env}
        return subprocess.run(
            [self.real_bash, str(LOCAL_FAST),
             '--changed-file', 'crates/agent-session/src/lib.rs'],
            cwd=REPO, env=env, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, check=False)

    def test_unreplaced_target_passes(self):
        result = self.run_local_fast()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertNotIn('shared target replaced', result.stdout)

    def test_replaced_binary_reports_the_replacement(self):
        result = self.run_local_fast(LOCAL_FAST_TEST_REPLACE='1')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('shared target replaced', result.stdout)
        self.assertIn('debug/agent-session', result.stdout)
        commands = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertIn(['test', '-p', 'nils-agent-session'], commands)


if __name__ == '__main__':
    unittest.main()
