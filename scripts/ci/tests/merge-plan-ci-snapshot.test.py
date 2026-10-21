#!/usr/bin/env python3
"""Offline regressions for immutable PR candidate versus lagging event base."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import unittest

spec = importlib.util.spec_from_file_location('snapshot_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class SnapshotTests(unittest.TestCase):
    setUp = fixtures.ConsumerExecutionTests.setUp
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute
    receipt = fixtures.ConsumerExecutionTests.receipt

    def advanced_main_candidate(self, change='source'):
        for path, source in {
            'scripts/ci/detect-docs-only.sh': 'printf "%s\\n" "$2" > "$DOCS_BASE_LOG"\necho false\n',
            'scripts/ci/detect-release-only.sh': 'echo false\n# Original base policy.\n',
        }.items():
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(source)
        self.candidate(change)
        self.original_candidate = self.merge
        self.git('checkout', '-q', '--detach', self.base)
        note = self.repo / 'docs/main-note.md'
        note.parent.mkdir(parents=True, exist_ok=True)
        note.write_text('Independent main change after event capture.\n')
        selector = self.repo / 'scripts/ci/lib/merge_plan.py'
        selector.write_text(selector.read_text() + '\n# New trusted-base snapshot.\n')
        (self.repo / 'scripts/ci/detect-release-only.sh').write_text('echo false\n# Actual candidate base policy.\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture main advanced')
        self.actual_base = self.git('rev-parse', 'HEAD')
        tree = self.git('merge-tree', '--write-tree', self.actual_base, self.head).splitlines()[0]
        self.merge = self.git('commit-tree', tree, '-p', self.actual_base, '-p', self.head,
                              '-m', 'fixture platform candidate')
        self.git('checkout', '-q', '--detach', self.merge)

    def detect_changes(self, head=None, candidate=None):
        workflow = (fixtures.ROOT / '.github/workflows/ci.yml').read_text()
        step = workflow.split('      - name: Detect change set\n', 1)[1]
        body = step.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
        script = '\n'.join(line[10:] for line in body.splitlines() if line.startswith('          '))
        env = dict(self.env, EVENT_NAME='pull_request', EVENT_SHA=candidate or self.merge,
                   PR_BASE_SHA=self.base, PR_HEAD_SHA=head or self.head,
                   HEAD_REF='fixture-topic', HEAD_REPOSITORY='fixture/repository',
                   REPOSITORY='fixture/repository', MERGE_GROUP_BASE_SHA='',
                   MERGE_GROUP_HEAD_SHA='', PUSH_BEFORE_SHA='', RUNNER_TEMP=str(self.root),
                   GITHUB_ENV=str(self.root / 'github-env'), DOCS_BASE_LOG=str(self.root / 'docs-base'))
        return subprocess.run(['bash', '-euo', 'pipefail', '-c', script], cwd=self.repo,
                              env=env, capture_output=True, text=True)

    def test_docs_and_release_routing_use_the_same_actual_snapshot(self):
        self.advanced_main_candidate()
        result = self.detect_changes()
        self.assertEqual(result.returncode, 0, result.stderr)
        output = Path(self.env['GITHUB_OUTPUT']).read_text()
        self.assertIn(f'base_sha={self.actual_base}\n', output)
        self.assertEqual((self.root / 'docs-base').read_text().strip(), self.actual_base)
        self.assertEqual((self.root / 'release-policy/detect-release-only.sh').read_text(),
                         'echo false\n# Actual candidate base policy.\n')

    def test_change_detection_holds_wrong_head_and_checkout_bindings(self):
        self.advanced_main_candidate()
        for kwargs in ({'head': self.actual_base}, {'candidate': self.original_candidate}):
            with self.subTest(kwargs=kwargs):
                result = self.detect_changes(**kwargs)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(Path(self.env['GITHUB_OUTPUT']).exists())
                self.assertFalse((self.root / 'docs-base').exists())

    def test_advanced_main_uses_actual_candidate_base_and_retains_event_provenance(self):
        self.advanced_main_candidate()
        refs = self.git('show-ref')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = self.receipt()
        self.assertEqual(receipt['event_base'], self.base)
        self.assertEqual(receipt['inputs'], {'base': self.actual_base, 'head': self.head,
                                           'candidate': self.merge,
                                           'tree': self.git('rev-parse', 'HEAD^{tree}')})
        self.assertEqual(receipt['checkout']['commit'], self.merge)
        self.assertEqual(receipt['validation_scope'], 'SCOPED')
        self.assertEqual(receipt['status'], 'PASS')
        self.assertTrue(any(row.get('args', [''])[0] == 'nextest' for row in rows))
        plan = json.loads((self.output / 'plan.json').read_text())
        actual_selector = self.git('show', f'{self.actual_base}:scripts/ci/lib/merge_plan.py') + '\n'
        self.assertEqual(plan['digests']['selector'], hashlib.sha256(actual_selector.encode()).hexdigest())
        self.assertEqual(self.git('show-ref'), refs)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.merge)

    def test_advanced_main_keeps_full_escalation_on_actual_snapshot(self):
        self.advanced_main_candidate('full')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['inputs']['base'], self.actual_base)
        self.assertEqual(self.receipt()['validation_scope'], 'FULL')
        self.assertEqual(json.loads((self.output / 'plan.json').read_text())['scope'], 'FULL')

    def test_wrong_reviewed_head_still_holds_after_main_advances(self):
        self.advanced_main_candidate()
        event = self.event('pull_request')
        event['pull_request']['head']['sha'] = self.actual_base
        result, rows = self.execute(event=event)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])
        self.assertFalse(self.output.exists())

    def test_checkout_cannot_substitute_another_candidate(self):
        self.advanced_main_candidate()
        self.git('checkout', '-q', '--detach', self.original_candidate)
        result, rows = self.execute()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])
        self.assertFalse(self.output.exists())

    def test_merge_group_base_remains_strict_when_pr_event_metadata_can_lag(self):
        self.advanced_main_candidate()
        result, rows = self.execute('merge_group')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])
        self.assertFalse(self.output.exists())
        event = self.event('merge_group')
        event['merge_group']['base_sha'] = self.actual_base
        result, _ = self.execute('merge_group', event)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.receipt()['inputs']['base'], self.actual_base)


if __name__ == '__main__':
    unittest.main(verbosity=2)
