#!/usr/bin/env python3
"""Independent selected coverage measurements and fail-closed aggregation."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('parallel_fixtures', Path(__file__).with_name('scoped-coverage-ratchet.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
COMPARE = ROOT / 'scripts/ci/scoped-coverage-compare.py'


class ParallelCoverageTests(unittest.TestCase):
    git = fixtures.RatchetTests.git
    candidate = fixtures.RatchetTests.candidate
    event = fixtures.RatchetTests.event
    execute = fixtures.RatchetTests.execute
    receipt = fixtures.RatchetTests.receipt
    setUp = fixtures.RatchetTests.setUp

    def side(self, side, **variables):
        self.log.unlink(missing_ok=True)
        result, calls = self.execute(env=dict(NILS_CLI_SCOPED_COVERAGE_SIDE=side,
            NILS_CLI_SCOPED_COVERAGE_IDENTITY='fixture-identity', **variables))
        stages = [row['stage'] for row in calls if row.get('args', [])[:2] == ['llvm-cov', 'nextest']]
        if result.returncode == 0:
            shutil.copytree(self.repo / 'target/ci-merge-plan/scoped-coverage', self.root / side, dirs_exist_ok=True)
        return result, stages, calls

    def compare(self, **variables):
        command = [sys.executable, str(COMPARE), '--base', self.base, '--candidate', self.merge,
                   '--baseline', str(self.root / 'base'), '--current', str(self.root / 'candidate'),
                   '--output', str(self.root / 'comparison.json'),
                   '--validation-receipt', str(self.root / 'validation.json')]
        (self.root / 'validation.json').write_text(json.dumps({'status': 'PASS', 'exit_code': 0}))
        return subprocess.run(command, capture_output=True, text=True, env=dict(os.environ, **variables))

    def test_base_worker_never_builds_or_tests_candidate(self):
        self.candidate()
        result, stages, calls = self.side('base', HEAD_FAIL='1')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(stages, ['BASE'])
        self.assertFalse(any(row.get('args', [''])[0] in ('clippy', 'build') for row in calls))
        self.assertEqual(json.loads((self.root / 'base/ratchet.json').read_text())['status'], 'MEASURED')

    def test_candidate_worker_never_builds_or_tests_base(self):
        self.candidate()
        result, stages, _ = self.side('candidate', BASE_FAIL='1')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(stages, ['HEAD'])
        self.assertEqual(json.loads((self.root / 'candidate/ratchet.json').read_text())['status'], 'MEASURED')

    def test_comparison_is_identical_to_the_sequential_ratchet(self):
        self.candidate()
        for old, old_lines, new, new_lines in [(80, 100, 80, 100), (80, 100, 82, 100),
                                               (95, 100, 94, 100),
                                               (800000000, 1000000000, 799999999, 1000000000)]:
            with self.subTest(base=(old, old_lines), candidate=(new, new_lines)):
                values = dict(BASE_HIT=str(old), BASE_FOUND=str(old_lines),
                              HEAD_HIT=str(new), HEAD_FOUND=str(new_lines))
                for side in ('base', 'candidate'):
                    result, _, _ = self.side(side, **values)
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                parallel = self.compare()
                sequential, _, _ = self.side('both', **values)
                self.assertEqual(parallel.returncode, sequential.returncode, parallel.stdout + parallel.stderr)
                comparison = json.loads((self.root / 'comparison.json').read_text())
                self.assertEqual(comparison['baseline'], {'covered': old, 'lines': old_lines})
                self.assertEqual(comparison['current'], {'covered': new, 'lines': new_lines})
                validation = json.loads((self.root / 'validation.json').read_text())
                self.assertEqual(validation['status'], comparison['status'])
                self.assertEqual(validation['exit_code'], parallel.returncode)

    def test_missing_failed_stale_or_mismatched_side_cannot_pass(self):
        self.candidate()
        for side in ('base', 'candidate'):
            result, _, _ = self.side(side)
            self.assertEqual(result.returncode, 0, result.stderr)
        receipt = self.root / 'candidate/ratchet.json'
        original = json.loads(receipt.read_text())
        variants = [dict(status='FAIL'), dict(candidate='0' * 40), dict(base='0' * 40),
                    dict(packages=['other']), dict(arguments=['-p', 'other']),
                    dict(coverage_identity='different'), dict(side='base'),
                    dict(current={'covered': 81, 'lines': 100})]
        for changes in variants:
            with self.subTest(changes=changes):
                receipt.write_text(json.dumps(dict(original, **changes)))
                self.assertNotEqual(self.compare().returncode, 0)
                self.assertEqual(json.loads((self.root / 'validation.json').read_text())['status'], 'FAIL')
                self.assertFalse((self.root / 'comparison.json').exists())
        receipt.unlink()
        self.assertNotEqual(self.compare().returncode, 0)
        receipt.write_text(json.dumps(original))
        (self.root / 'candidate/lcov.info').unlink()
        self.assertNotEqual(self.compare().returncode, 0)

    def test_side_test_failure_stays_a_failure(self):
        self.candidate()
        for side, failure, expected in [('base', 'BASE_FAIL', ['BASE']),
                                        ('candidate', 'HEAD_FAIL', ['HEAD'])]:
            result, stages, _ = self.side(side, **{failure: '1'})
            self.assertEqual(result.returncode, 17, result.stdout + result.stderr)
            self.assertEqual(stages, expected)


if __name__ == '__main__':
    unittest.main(verbosity=2)
