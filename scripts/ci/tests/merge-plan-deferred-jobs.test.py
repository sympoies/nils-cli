#!/usr/bin/env python3
"""Required job states for an early trusted PR scope decision."""
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import unittest

spec = importlib.util.spec_from_file_location('deferred_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


def job_body(name):
    body = (fixtures.ROOT / '.github/workflows/ci.yml').read_text().split(f'\n  {name}:\n', 1)[1]
    return re.split(r'\n  [a-z_]+:\n', body, maxsplit=1)[0]


def job_runs(name, **values):
    expression = next(line[8:] for line in job_body(name).splitlines() if line.startswith('    if: '))
    expression = expression.removeprefix('${{ ').removesuffix(' }}')
    defaults = {'github.event_name': 'pull_request', 'needs.changes.result': 'success',
                'needs.changes.outputs.validation_scope': 'FULL', 'needs.test.result': 'success',
                'needs.test.outputs.validation_scope': 'FULL', 'needs.test_macos.result': 'skipped',
                'needs.test_containment.result': 'success', 'needs.coverage.result': 'skipped'}
    defaults.update(values)
    for key in sorted(defaults, key=len, reverse=True):
        expression = expression.replace(key, repr(defaults[key]))
    expression = expression.replace('!cancelled()', 'True').replace('always()', 'True')
    return eval(expression.replace('&&', 'and').replace('||', 'or').replace('!(', 'not ('),
                {'__builtins__': {}}, {})


class DeferredJobTests(unittest.TestCase):
    def test_full_pr_required_jobs_are_skipped_instead_of_success(self):
        for job in ('test_macos', 'coverage'):
            with self.subTest(job=job):
                self.assertFalse(job_runs(job), f'{job} would report success for deferred full PR work')

    def test_scoped_pr_and_merge_group_keep_executing_required_jobs(self):
        for event, scope in (('pull_request', 'SCOPED'), ('merge_group', ''), ('schedule', '')):
            for job in ('test_macos', 'coverage'):
                with self.subTest(event=event, scope=scope, job=job):
                    self.assertTrue(job_runs(job, **{'github.event_name': event,
                        'needs.changes.outputs.validation_scope': scope,
                        'needs.test_macos.result': 'success'}))

    def test_failed_upstreams_cannot_skip_the_required_coverage_guard(self):
        for lane in ('changes', 'test', 'test_containment'):
            for state in ('failure', 'skipped', 'cancelled'):
                with self.subTest(lane=lane, state=state):
                    self.assertTrue(job_runs('coverage', **{f'needs.{lane}.result': state}))

    def test_scoped_linux_with_deferred_macos_runs_the_failure_guard(self):
        self.assertTrue(job_runs('coverage', **{'needs.test.outputs.validation_scope': 'SCOPED'}))
        step = job_body('coverage').split('      - name: Require the upstream lanes\n', 1)[1]
        script = step.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
        script = '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))
        result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], capture_output=True,
            env={'PATH': '/usr/bin:/bin', 'CHANGES_RESULT': 'success', 'TEST_RESULT': 'success',
                 'TEST_MACOS_RESULT': 'skipped', 'TEST_CONTAINMENT_RESULT': 'success'})
        self.assertNotEqual(result.returncode, 0)

    def test_only_genuine_deferred_checks_receive_the_visible_title(self):
        self.assertTrue(job_runs('deferred_full'))
        for lane in ('changes', 'test', 'test_containment'):
            for state in ('failure', 'skipped', 'cancelled'):
                self.assertFalse(job_runs('deferred_full', **{f'needs.{lane}.result': state}))
        self.assertFalse(job_runs('deferred_full', **{'needs.test.outputs.validation_scope': 'SCOPED'}))
        result = subprocess.run(['bash', '-euo', 'pipefail', '-c',
            'source scripts/ci/lib/node-scope.sh\nnode_scoped node --test scripts/ci/tests/deferred-full-checks.test.cjs'],
            cwd=fixtures.ROOT, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_macos_timeout_keeps_margin_for_pr_and_full_lanes(self):
        line = next(line for line in job_body('test_macos').splitlines() if 'timeout-minutes:' in line)
        expression = line.split('timeout-minutes:', 1)[1].strip().removeprefix('${{ ').removesuffix(' }}')
        for event, minutes in (('pull_request', 75), ('merge_group', 75), ('schedule', 75)):
            observed = eval(expression.replace('github.event_name', repr(event))
                .replace('&&', 'and').replace('||', 'or'), {'__builtins__': {}}, {})
            self.assertEqual(observed, minutes)


class PreviewTests(unittest.TestCase):
    setUp = fixtures.ConsumerExecutionTests.setUp
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute

    def test_trusted_preview_emits_scope_without_executing_tests(self):
        self.candidate()
        for variables, scope in (({}, 'SCOPED'), ({'FAIL_PLAN_METADATA': '1'}, 'FULL')):
            with self.subTest(scope=scope):
                result, rows = self.execute(env=variables, extra_args=['--plan-only'])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(rows, [])
                summary = json.loads((self.output / 'planning.json').read_text())
                self.assertEqual(summary['status'], 'PLANNED')
                self.assertEqual(summary['validation_scope'], scope)
                self.assertEqual(summary['inputs']['candidate'], self.merge)
                self.assertFalse((self.output / 'validation.json').exists())
                self.assertEqual((self.root / 'github-output').read_text(), f'validation_scope={scope}\n')

    def test_merge_group_cannot_use_a_preview_in_place_of_validation(self):
        self.candidate()
        result, rows = self.execute('merge_group', extra_args=['--plan-only'])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
