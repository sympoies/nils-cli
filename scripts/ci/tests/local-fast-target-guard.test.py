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
# Cargo resolver 2 unifies features across the -p set, so the same package built
# in a bigger set links a different executable. A build whose identity already
# matches is fresh and leaves the file alone.
packages = [args[i + 1] for i, arg in enumerate(args) if arg == "-p"]
identity = 1_000_000_000 * max(len(packages), 1)
names = {{"nils-agent-session": "agent_session", "nils-memo": "memo"}}
for package in packages:
    stem = names.get(package)
    if stem is None:
        continue
    binary = stem.replace("_", "-")
    for path, content in ((os.path.join(debug, binary), "built"),
                          (os.path.join(debug, "deps", stem + "-0123abcd"), "test")):
        if not os.path.exists(path) or os.stat(path).st_mtime_ns != identity:
            write(path, content, identity)
# The test run itself: another lane rebuilds the shared binary mid-run.
if "--no-run" not in args and args[:1] == ["test"]:
    if os.environ.get("LOCAL_FAST_TEST_REPLACE") == "1":
        write(os.path.join(debug, "agent-session"), "built-from-another-worktree", 2_000_000_000)
    raise SystemExit(int(os.environ.get("LOCAL_FAST_TEST_EXIT", "0")))
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

    def run_local_fast(self, changed=('crates/agent-session/src/lib.rs',), **extra_env):
        env = {**self.env, **extra_env}
        changed_args = [arg for path in changed for arg in ('--changed-file', path)]
        return subprocess.run(
            [self.real_bash, str(LOCAL_FAST), *changed_args],
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

    def test_failing_tests_against_a_replaced_binary_report_the_replacement(self):
        # The #2697 symptom: a product test fails because its binary was replaced.
        result = self.run_local_fast(
            LOCAL_FAST_TEST_REPLACE='1', LOCAL_FAST_TEST_EXIT='101')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('shared target replaced', result.stdout)
        self.assertIn('debug/agent-session', result.stdout)

    def test_failing_tests_on_an_intact_target_are_a_product_failure(self):
        result = self.run_local_fast(LOCAL_FAST_TEST_EXIT='101')
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('cargo test -p nils-agent-session', result.stdout)
        self.assertNotIn('shared target replaced', result.stdout)

    def test_multi_package_selection_is_not_a_replacement(self):
        # A solo package build can relink an executable the combined build made
        # with unified features; that is not another lane replacing it.
        result = self.run_local_fast(changed=(
            'crates/agent-session/src/lib.rs', 'crates/memo/src/lib.rs'))
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertNotIn('shared target replaced', result.stdout)
        commands = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertIn(['test', '-p', 'nils-agent-session'], commands)
        self.assertIn(['test', '-p', 'nils-memo'], commands)


if __name__ == '__main__':
    unittest.main()
