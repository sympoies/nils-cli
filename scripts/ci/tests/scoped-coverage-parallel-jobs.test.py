#!/usr/bin/env python3
"""Workflow routing for concurrent measurements and the required aggregate."""
import importlib.util
import json
import os
import subprocess
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('parallel_jobs', Path(__file__).with_name('merge-plan-deferred-jobs.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class ParallelJobTests(unittest.TestCase):
    def test_only_scoped_prs_split_into_concurrent_sides(self):
        worker = fixtures.job_body('test_macos_sides')
        self.assertIn('needs: changes', worker)
        self.assertIn('fail-fast: false', worker)
        line = next(line for line in worker.splitlines() if 'side: ${{ fromJSON(' in line)
        expression = line.split('fromJSON(', 1)[1].removesuffix(') }}')
        for event, scope, expected in [('pull_request', 'SCOPED', ['base', 'candidate']),
                                       ('pull_request', 'FULL', ['both']),
                                       ('merge_group', '', ['both']), ('schedule', '', ['both']),
                                       ('workflow_dispatch', '', ['both'])]:
            value = expression.replace('github.event_name', repr(event)).replace('needs.changes.outputs.validation_scope', repr(scope))
            sides = json.loads(eval(value.replace('&&', 'and').replace('||', 'or'), {'__builtins__': {}}, {}))
            self.assertEqual(sides, expected)
        self.assertIn('NILS_CLI_SCOPED_COVERAGE_SIDE: ${{ matrix.side }}', worker)
        self.assertIn('matrix.side', worker.split('name: merge-validation-macos-', 1)[1])
        self.assertIn("matrix.side != 'base'", worker.split('      - name: Upload coverage LCOV', 1)[1].split('uses:', 1)[0])

    def test_required_name_remains_aggregate_with_failure_guard(self):
        aggregate = fixtures.job_body('test_macos')
        self.assertIn('needs: [changes, test_macos_sides]', aggregate)
        self.assertIn('always()', aggregate)
        script = aggregate.split('      - name: Require macOS workers\n', 1)[1].split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
        script = '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))
        for status in ('failure', 'cancelled', 'skipped', 'success'):
            result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], env=dict(os.environ, MACOS_RESULT=status), capture_output=True)
            self.assertEqual(result.returncode == 0, status == 'success')
        self.assertIn('scoped-coverage-compare.py', aggregate)
        self.assertIn('needs.changes.outputs.base_sha', aggregate)
        self.assertIn('github.sha', aggregate)
        self.assertFalse(fixtures.job_runs('test_macos'))
        self.assertTrue(fixtures.job_runs('test_macos', **{'needs.changes.outputs.validation_scope': 'SCOPED'}))


if __name__ == '__main__':
    unittest.main(verbosity=2)
