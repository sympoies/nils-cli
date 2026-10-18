#!/usr/bin/env python3
"""PR scope, deferred full macOS, and merge-queue safety-net contracts."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('lane_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
spec = importlib.util.spec_from_file_location('ratchet_fixtures', Path(__file__).with_name('scoped-coverage-ratchet.test.py'))
ratchet_fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ratchet_fixtures)


class PrLaneTests(unittest.TestCase):
    setUp = fixtures.ConsumerExecutionTests.setUp
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute
    receipt = fixtures.ConsumerExecutionTests.receipt

    def test_full_macos_pr_is_explicitly_deferred_without_executing_full_tests(self):
        self.candidate('full')
        result, rows = self.execute(env={'NILS_CLI_PR_DEFER_FULL': '1', 'NILS_CLI_TEST_RUNNER': 'llvm-cov'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [], 'full macOS tests still ran in the PR lane')
        receipt = self.receipt()
        self.assertEqual(receipt['status'], 'DEFERRED')
        self.assertEqual(receipt['schema_version'], 'nils-cli.merge-validation.v2')
        self.assertEqual(receipt['validation_scope'], 'DEFERRED_FULL')
        self.assertEqual(receipt['command'], [])
        self.assertIsNone(receipt['coverage_policy'])
        self.assertEqual((self.root / 'github-output').read_text(), 'validation_scope=DEFERRED_FULL\n')

    def test_metadata_hold_is_retained_when_full_macos_work_is_deferred(self):
        self.candidate()
        result, rows = self.execute(env={'NILS_CLI_PR_DEFER_FULL': '1', 'NILS_CLI_TEST_RUNNER': 'llvm-cov',
                                        'FAIL_PLAN_METADATA': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [])
        self.assertEqual(self.receipt()['fallback'], 'planner_hold')
        self.assertEqual(self.receipt()['status'], 'DEFERRED')

    def test_linux_and_merge_group_cannot_defer_full_work(self):
        self.candidate('full')
        for event, runner in (('pull_request', 'nextest'), ('merge_group', 'llvm-cov')):
            with self.subTest(event=event, runner=runner):
                self.log.unlink(missing_ok=True)
                result, rows = self.execute(event, env={'NILS_CLI_PR_DEFER_FULL': '1',
                                                        'NILS_CLI_TEST_RUNNER': runner})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(rows, [{'kind': 'full'}])
                self.assertEqual(self.receipt()['status'], 'PASS')
                self.assertEqual(self.receipt()['validation_scope'], 'FULL')

    def test_invalid_candidate_binding_still_fails_before_deferral(self):
        self.candidate('full')
        event = self.event('pull_request')
        event['pull_request']['head']['sha'] = 'a' * 40
        result, rows = self.execute(event=event, env={'NILS_CLI_PR_DEFER_FULL': '1',
                                                    'NILS_CLI_TEST_RUNNER': 'llvm-cov'})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])
        self.assertFalse((self.output / 'validation.json').exists())


class CoverageLaneTests(unittest.TestCase):
    def test_scoped_linux_requires_real_scoped_macos_coverage(self):
        workflow = (fixtures.ROOT / '.github/workflows/ci.yml').read_text()
        step = workflow.split('      - name: Require selected PR coverage\n', 1)[1]
        script = step.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
        script = '\n'.join(line[10:] for line in script.splitlines())
        for linux, macos, succeeds in (('SCOPED', 'SCOPED', True), ('SCOPED', 'DEFERRED_FULL', False),
                                       ('SCOPED', '', False), ('FULL', 'DEFERRED_FULL', True)):
            with self.subTest(linux=linux, macos=macos), tempfile.TemporaryDirectory() as directory:
                result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], cwd=directory,
                    env=dict(os.environ, LINUX_SCOPE=linux, MACOS_SCOPE=macos), capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, succeeds, result.stderr)


class ScopedPrRatchetTests(unittest.TestCase):
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute
    receipt = fixtures.ConsumerExecutionTests.receipt
    setUp = ratchet_fixtures.RatchetTests.setUp

    def test_scoped_pr_keeps_the_instrumented_base_coverage_ratchet(self):
        self.candidate()
        result, rows = self.execute(env={'NILS_CLI_PR_DEFER_FULL': '1', 'NILS_CLI_TEST_RUNNER': 'llvm-cov'})
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = self.receipt()
        self.assertEqual(receipt['status'], 'PASS')
        self.assertEqual(receipt['validation_scope'], 'SCOPED')
        self.assertEqual(receipt['coverage_policy'], 'base-no-decrease')
        self.assertEqual(receipt['coverage_base'], self.base)
        self.assertEqual([row['stage'] for row in rows if row.get('args', [])[:2] == ['llvm-cov', 'nextest']],
                         ['BASE', 'HEAD'])

    def test_scoped_decrease_still_fails_with_deferral_enabled(self):
        self.candidate()
        result, _ = self.execute(env={'NILS_CLI_PR_DEFER_FULL': '1', 'BASE_HIT': '95', 'HEAD_HIT': '94'})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.receipt()['status'], 'FAIL')
        self.assertEqual((self.root / 'github-output').read_text() if (self.root / 'github-output').exists() else '', '')


if __name__ == '__main__':
    unittest.main(verbosity=2)
