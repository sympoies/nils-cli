#!/usr/bin/env python3
"""Offline macOS queue selection and Clippy execution regressions."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('ci_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class MacosClippyTests(unittest.TestCase):
    setUp = fixtures.ConsumerExecutionTests.setUp
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    squash_candidate = fixtures.ConsumerExecutionTests.squash_candidate

    def execute(self, name='merge_group', env=None, event=None):
        payload = self.root / 'event.json'
        payload.write_text(json.dumps(self.event(name) if event is None else event))
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/ci/nils-cli-macos-clippy.py'),
                                 '--repo', str(self.repo), '--event-name', name,
                                 '--event-path', str(payload), '--candidate', self.merge,
                                 '--output', str(self.output)],
                                env=dict(self.env, **(env or {})), capture_output=True, text=True)
        rows = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        self.assertFalse(Path(str(self.log) + '.compiler').exists())
        return result, rows

    def test_test_only_helper_change_checks_affected_packages_and_integration_targets(self):
        source = self.repo / 'crates/selected/src/test_support.rs'
        source.write_text('// original test helper\n')
        self.candidate()
        # Model a cfg(test) helper with Linux-only callers, as in the regression.
        source.write_text('#[cfg(test)] fn helper() {}\n'
                          '#[cfg(all(test, target_os = "linux"))]\n'
                          'mod tests { #[test] fn caller() { super::helper(); } }\n')
        (self.repo / 'crates/selected/src/lib.rs').write_text('// synthetic fixture\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture test-only helper')
        self.head = self.git('rev-parse', 'HEAD')
        self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                              '-p', self.base, '-p', self.head, '-m', 'fixture candidate')
        self.git('checkout', '-q', '--detach', self.merge)
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows[0]['args'], ['clippy', '--locked', '--all-targets', '--all-features',
                                          '-p', 'consumer', '-p', 'selected', '--', '-D', 'warnings'])
        self.assertEqual(len(rows), 1, 'compile/lint only; no tests or coverage')

    def test_integration_test_source_selects_its_owner_and_dependents(self):
        tests = self.repo / 'crates/selected/tests'
        tests.mkdir()
        integration = tests / 'integration.rs'
        integration.write_text('// original integration test\n')
        self.candidate()
        integration.write_text('// changed integration test\n')
        (self.repo / 'crates/selected/src/lib.rs').write_text('// synthetic fixture\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture integration target')
        self.head = self.git('rev-parse', 'HEAD')
        self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                              '-p', self.base, '-p', self.head, '-m', 'fixture integration candidate')
        self.git('checkout', '-q', '--detach', self.merge)
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('selected', rows[0]['args'])
        self.assertIn('consumer', rows[0]['args'])
        self.assertIn('--all-targets', rows[0]['args'])

    def test_docs_group_still_runs_workspace_clippy(self):
        (self.repo / 'README.md').write_text('Original fixture documentation\n')
        self.candidate()
        (self.repo / 'crates/selected/src/lib.rs').write_text('// synthetic fixture\n')
        (self.repo / 'README.md').write_text('Changed fixture documentation\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture docs-only head')
        self.head = self.git('rev-parse', 'HEAD')
        self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                              '-p', self.base, '-p', self.head, '-m', 'fixture docs candidate')
        self.git('checkout', '-q', '--detach', self.merge)
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--workspace', rows[0]['args'])

    def test_scope_uncertainty_checks_the_workspace(self):
        self.candidate('full')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--workspace', rows[0]['args'])

    def test_unavailable_metadata_checks_the_workspace(self):
        self.candidate()
        result, rows = self.execute(env={'FAIL_PLAN_METADATA': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--workspace', rows[0]['args'])

    def test_release_version_bump_checks_the_workspace(self):
        manifest = self.repo / 'Cargo.toml'
        manifest.write_text('[workspace]\nmembers = []\n[workspace.package]\nversion = "0.1.0"\n')
        self.candidate()
        (self.repo / 'crates/selected/src/lib.rs').write_text('// synthetic fixture\n')
        manifest.write_text(manifest.read_text().replace('0.1.0', '0.1.1'))
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture release bump')
        self.head = self.git('rev-parse', 'HEAD')
        self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                              '-p', self.base, '-p', self.head, '-m', 'fixture release candidate')
        self.git('checkout', '-q', '--detach', self.merge)
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--workspace', rows[0]['args'])

    def test_squash_queue_retains_affected_scope(self):
        self.squash_candidate()
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('--workspace', rows[0]['args'])
        self.assertIn('consumer', rows[0]['args'])

    def test_clippy_failure_blocks_and_is_recorded(self):
        self.candidate()
        result, _ = self.execute(env={'FAIL_CARGO_COMMAND': 'clippy'})
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(json.loads((self.output / 'validation.json').read_text())['status'], 'FAIL')

    def test_event_mismatch_holds_before_cargo(self):
        self.candidate()
        event = self.event('merge_group')
        event['merge_group']['head_sha'] = self.head
        result, rows = self.execute(event=event)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])

    def test_pr_does_not_execute_clippy(self):
        self.candidate()
        result, rows = self.execute('pull_request')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [])


if __name__ == '__main__':
    unittest.main(verbosity=2)
