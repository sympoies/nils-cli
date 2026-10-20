#!/usr/bin/env python3
"""Diagnostic preview remains distinct from executed candidate validation."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('preview_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


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
