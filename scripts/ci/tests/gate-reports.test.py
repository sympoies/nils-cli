#!/usr/bin/env python3
"""Report routing regressions without workspace builds."""
import os
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('gate_reports', ROOT / 'scripts/ci/gate-reports.py')
reports_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reports_module)


class GateReports(unittest.TestCase):
    def test_default_is_unique_and_independent_of_cargo_target(self):
        with tempfile.TemporaryDirectory() as directory:
            env = {'XDG_STATE_HOME': directory, 'CARGO_TARGET_DIR': str(ROOT / 'target')}
            with patch.dict(os.environ, env, clear=True), \
                 patch.object(reports_module.subprocess, 'call', return_value=23) as launch:
                self.assertEqual(reports_module.run(['cargo', 'nextest', 'run']), 23)
                first = launch.call_args.kwargs['env']['NEXTEST_STATE_DIR']
                self.assertTrue(Path(first).is_relative_to(Path(directory)))
                self.assertFalse(Path(first).is_relative_to(ROOT))
                self.assertEqual(reports_module.run(['cargo', 'nextest', 'run']), 23)
                self.assertNotEqual(first, launch.call_args.kwargs['env']['NEXTEST_STATE_DIR'])

    def test_target_override_is_refused_before_launch(self):
        with patch.dict(os.environ, {'NILS_CLI_REPORT_DIR': str(ROOT / 'target/reports')}), \
             patch.object(reports_module.subprocess, 'call') as launch:
            with self.assertRaisesRegex(ValueError, 'outside Cargo target'):
                reports_module.run(['cargo', 'nextest', 'run'])
            launch.assert_not_called()

    def test_nextest_and_coverage_write_outside_target(self):
        for command in ('nextest run --profile ci', 'llvm-cov nextest --profile ci',
                        'nextest run --profile ci -- --exact',
                        'llvm-cov nextest --profile ci -- --exact'):
            with self.subTest(command=command), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                cargo = root / 'cargo'
                cargo.write_text('''#!/usr/bin/env python3
import os, sys, tomllib
from pathlib import Path
args = sys.argv
if '--' in args:
    args = args[:args.index('--')]
if '--tool-config-file' in args:
    config = Path(args[args.index('--tool-config-file') + 1].split(':', 1)[1])
    store = Path(tomllib.loads(config.read_text())['store']['dir'])
else:
    store = Path(os.environ['CARGO_TARGET_DIR']) / 'nextest'
report = store / 'ci/junit.xml'
report.parent.mkdir(parents=True, exist_ok=True)
report.write_text('<testsuites/>')
''')
                cargo.chmod(0o755)
                reports = root / 'owner reports'
                cache = root / 'cache'
                env = dict(os.environ, NILS_CLI_REPORT_DIR=str(reports),
                           CARGO_TARGET_DIR=str(cache))
                result = subprocess.run(['bash', '-c',
                    'source scripts/ci/test-env.sh; run() { "$@"; }; run_test "$@"',
                    'fixture', str(cargo), *command.split()],
                    cwd=ROOT, env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue((reports / 'nextest/ci/junit.xml').is_file(), result.stdout)
                self.assertEqual((reports / 'nextest/ci/junit.xml').read_text(), '<testsuites/>')
                self.assertFalse(cache.exists())
                for workflow in ('ci.yml', 'full-validation.yml'):
                    text = (ROOT / '.github/workflows' / workflow).read_text()
                    probe = text.split('      - name: Locate JUnit report\n', 1)[1]
                    probe = probe.split('        run: |\n', 1)[1].split('\n\n', 1)[0]
                    script = '\n'.join(line[10:] for line in probe.splitlines())
                    output = root / 'github-output'
                    output.unlink(missing_ok=True)
                    result = subprocess.run(['bash', '-eu', '-c', script],
                        env=dict(env, GITHUB_OUTPUT=str(output)), capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(output.read_text(), 'present=true\n')

    def test_ci_consumers_use_the_external_report_path(self):
        for workflow in ('ci.yml', 'full-validation.yml'):
            text = (ROOT / '.github/workflows' / workflow).read_text()
            self.assertIn('NILS_CLI_REPORT_DIR=$RUNNER_TEMP/nils-cli-reports', text)
            self.assertNotIn('target/nextest/ci/junit.xml', text)
            self.assertNotIn('target/**/nextest/ci/junit.xml', text)
            self.assertIn('report_paths: ${{ env.NILS_CLI_REPORT_DIR }}/nextest/ci/junit.xml', text)
            self.assertIn('path: ${{ env.NILS_CLI_REPORT_DIR }}/nextest/ci/junit.xml', text)
            self.assertIn('steps.junit-report.outputs.present', text)


if __name__ == '__main__':
    unittest.main()
