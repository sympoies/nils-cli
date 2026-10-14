#!/usr/bin/env python3
"""Metadata failure contracts: mocked tools and offline fake-Cargo snapshots."""
import errno
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
from types import SimpleNamespace
import unittest
from unittest import mock

import jsonschema

spec = importlib.util.spec_from_file_location('metadata_fixtures', Path(__file__).with_name('merge-plan.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
spec = importlib.util.spec_from_file_location('metadata_selector', fixtures.LIB)
selector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(selector)


class MetadataFailureTests(unittest.TestCase):
    def setUp(self):
        self.root = Path('/fixture/repository')
        self.environment = mock.patch.dict(os.environ, {}, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.tools = mock.patch.object(selector.shutil, 'which', side_effect=lambda name: '/fixture/bin/' + name)
        self.tools.start()
        self.addCleanup(self.tools.stop)

    def failure(self, kind, process):
        with mock.patch.object(selector.subprocess, 'run', side_effect=process) as run:
            with self.assertRaises(selector.PlanError) as caught:
                selector.metadata(self.root, 'base_metadata', self.root)
        error = caught.exception
        self.assertEqual((error.code, error.stage), ('METADATA_FAILED', 'base_metadata'))
        recovery = getattr(error, 'recovery', {})
        self.assertEqual(recovery.get('kind'), kind, 'metadata failure cause was discarded')
        return recovery, run

    def test_missing_tool_identifies_cargo_or_rustc_without_execution(self):
        for missing in ('cargo', 'rustc'):
            with self.subTest(tool=missing), mock.patch.object(selector.shutil, 'which',
                    side_effect=lambda name: None if name == missing else '/fixture/bin/' + name):
                recovery, run = self.failure('missing_tool', AssertionError('must not execute'))
                self.assertEqual(recovery['tool'], missing)
                run.assert_not_called()

    def test_spawn_error_retains_errno_without_exception_path(self):
        recovery, _ = self.failure('spawn_error', OSError(errno.EACCES, 'fixture failure', '/example/private/tool'))
        self.assertEqual(recovery['errno'], errno.EACCES)
        self.assertNotIn('/example/private', json.dumps(recovery))

    def test_cargo_exit_retains_status_and_useful_bounded_diagnostic(self):
        failure = subprocess.CalledProcessError(101, ['fixture-cargo'],
            stderr=b'error: no matching package named fixture-dependency found\n')
        recovery, _ = self.failure('cargo_exit', failure)
        self.assertEqual(recovery['exit_code'], 101)
        self.assertIn('offline registry cache', recovery['stderr_tail'])

    def test_invalid_json_never_copies_stdout(self):
        recovery, _ = self.failure('json_invalid',
            lambda *args, **kwargs: SimpleNamespace(stdout=b'fixture-private-json-content'))
        self.assertNotIn('fixture-private-json-content', json.dumps(recovery))

    def test_workspace_root_mismatch_is_distinct_without_copying_root(self):
        raw = json.dumps(fixtures.graph(Path('/example/unexpected/root'))).encode()
        recovery, _ = self.failure('workspace_root_mismatch',
            lambda *args, **kwargs: SimpleNamespace(stdout=raw))
        self.assertNotIn('/example/unexpected/root', json.dumps(recovery))

    def test_invalid_resolved_graph_is_distinct(self):
        value = fixtures.graph(self.root)
        value['resolve']['nodes'].pop()
        self.failure('graph_error', lambda *args, **kwargs: SimpleNamespace(stdout=json.dumps(value).encode()))

    def test_cargo_diagnostic_redacts_environment_and_private_identifiers(self):
        environment = {'TEST_AUTH_TOKEN': 'fixture-token-value', 'HOME': '/example/home',
                       'USER': 'fixture-person', 'HOSTNAME': 'fixture-host',
                       'AGENT_SESSION_ID': '00000000-1111-2222-3333-444444444444'}
        stderr = ('\x1b[31merror: fixture-person fixture-host /example/home/cache/private-file\x1b[0m\n'
                  'fixture-token-value password=fixture-password Bearer fixture-bearer\n'
                  'https://fixture-user:fixture-password@example.invalid/path?token=fixture-query\n'
                  'fixture@example.invalid 192.0.2.1 C:\\example\\private\\file\n'
                  'proxy.example.invalid "/example/Folder With Spaces/file" [2001:db8::1]\n'
                  '00000000-1111-2222-3333-444444444444\n').encode()
        with mock.patch.dict(os.environ, environment):
            recovery, _ = self.failure('cargo_exit', subprocess.CalledProcessError(101, ['fixture'], stderr=stderr))
        tail = recovery['stderr_tail']
        for private in (*environment.values(), 'fixture-password', 'fixture-bearer', 'fixture-query',
                        'fixture@example.invalid', '192.0.2.1', 'C:\\example', '/example/home', '\x1b'):
            self.assertNotIn(private, tail)
        for private in ('proxy.example.invalid', 'Folder With Spaces', '2001:db8::1'):
            self.assertNotIn(private, tail)
        self.assertIn('error:', tail)
        self.assertLessEqual(len(tail.encode()), 2048)

    def test_stderr_tail_is_bounded_after_redaction_and_utf8_safe(self):
        stderr = ('long fixture noise\n' * 1000 + 'é' * 1100 + '\nerror: dependency unavailable\n').encode()
        recovery, _ = self.failure('cargo_exit', subprocess.CalledProcessError(101, ['fixture'], stderr=stderr))
        tail = recovery['stderr_tail']
        self.assertLessEqual(len(tail.encode()), 2048)
        self.assertTrue(tail.endswith('error: <redacted diagnostic line>\n'))

    def test_unknown_diagnostics_never_publish_source_derived_prose(self):
        raw = (b'error at /example/First Last/Private Project/Cargo.toml\n'
               b'error at C:\\example\\First Last\\Private Project\\Cargo.toml\n'
               b'error: private-registry-alias private-host-identifier unlabeled-fixture-credential\n')
        recovery, _ = self.failure('cargo_exit', subprocess.CalledProcessError(101, ['fixture'], stderr=raw))
        self.assertEqual(recovery['stderr_tail'],
                         'diagnostic: <redacted diagnostic line>\n' * 2 +
                         'error: <redacted diagnostic line>\n')

    def test_success_keeps_offline_all_platform_graph_and_trusted_tooling(self):
        raw = json.dumps(fixtures.graph(self.root)).encode()
        with mock.patch.object(selector.subprocess, 'run', return_value=SimpleNamespace(stdout=raw)) as run:
            value, digest = selector.metadata(self.root, 'base_metadata', self.root)
        self.assertEqual(value, json.loads(raw))
        self.assertEqual(len(digest), 64)
        command = run.call_args.args[0]
        self.assertEqual(command, ['/fixture/bin/cargo', 'metadata', '--manifest-path',
                                  '/fixture/repository/Cargo.toml', '--offline', '--locked',
                                  '--all-features', '--format-version', '1'])
        self.assertEqual(run.call_args.kwargs['cwd'], self.root)
        environment = run.call_args.kwargs['env']
        self.assertEqual(environment['RUSTUP_AUTO_INSTALL'], '0')
        self.assertEqual(environment['RUSTC'], '/fixture/bin/rustc')
        self.assertEqual(environment['RUSTC_WRAPPER'], '')
        self.assertEqual(environment['RUSTC_WORKSPACE_WRAPPER'], '')


class MetadataPlanTests(unittest.TestCase):
    setUp = fixtures.CommandTests.setUp
    git = fixtures.CommandTests.git
    commit = fixtures.CommandTests.commit
    run_plan = fixtures.CommandTests.run_plan

    def test_new_recovery_is_versioned_and_legacy_v1_contract_remains_closed(self):
        result = self.run_plan(FAIL_METADATA='1')
        plan = json.loads(result.stdout)
        self.assertEqual(plan['schema_version'], 'nils-cli.merge-plan.v2')
        legacy = json.loads((fixtures.ROOT / 'scripts/ci/merge-plan.schema.v1.json').read_text())
        with self.assertRaises(jsonschema.ValidationError):
            jsonschema.validate(plan, legacy)

    def test_ci_consumer_accepts_legacy_and_current_plans_but_holds_unknown_versions(self):
        spec = importlib.util.spec_from_file_location('metadata_consumer',
                    fixtures.ROOT / 'scripts/ci/nils-cli-merge-plan-checks.py')
        consumer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(consumer)
        result = self.run_plan()
        self.assertEqual(result.returncode, 0, result.stderr)
        plan = json.loads(result.stdout)
        for version in ('nils-cli.merge-plan.v1', 'nils-cli.merge-plan.v2', 'nils-cli.merge-plan.v3'):
            with self.subTest(version=version):
                plan['schema_version'] = version
                unsigned = dict(plan, digests={key: value for key, value in plan['digests'].items() if key != 'plan'})
                plan['digests']['plan'] = hashlib.sha256(json.dumps(unsigned, sort_keys=True,
                    separators=(',', ':'), ensure_ascii=True).encode()).hexdigest()
                if version.endswith('v3'):
                    with self.assertRaises(ValueError):
                        consumer.plan_scope(plan, plan['inputs'])
                else:
                    self.assertEqual(consumer.plan_scope(plan, plan['inputs']), ('SCOPED', ['leaf'], None))

    def test_base_and_candidate_diagnostics_survive_schema_and_plan_hash(self):
        for variables, stage in (({'FAIL_METADATA': '1'}, 'base_metadata'),
                                 ({'FAIL_CANDIDATE': '1'}, 'candidate_metadata')):
            with self.subTest(stage=stage):
                result = self.run_plan(**variables)
                self.assertEqual(result.returncode, 1)
                plan = json.loads(result.stdout)
                self.assertEqual((plan['status'], plan['action'], plan['scope']), ('PLAN_ERROR', 'HOLD', None))
                self.assertEqual(plan['selected_packages'], [])
                self.assertEqual(plan['error']['stage'], stage)
                self.assertEqual(plan['error']['recovery'].get('kind'), 'cargo_exit',
                                 'serialized HOLD plan discarded the original failure cause')
                self.assertEqual(plan['error']['recovery']['exit_code'], 101)
                self.assertNotIn('/private/fixture', result.stdout)
                self.assertNotIn(self.temp.name, result.stdout)
                jsonschema.validate(plan, json.loads(fixtures.SCHEMA.read_text()))
                unsigned = dict(plan, digests={key: value for key, value in plan['digests'].items() if key != 'plan'})
                digest = hashlib.sha256(json.dumps(unsigned, sort_keys=True, separators=(',', ':'),
                                                   ensure_ascii=True).encode()).hexdigest()
                self.assertEqual(plan['digests']['plan'], digest)


if __name__ == '__main__':
    unittest.main(verbosity=2)
