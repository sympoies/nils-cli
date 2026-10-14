#!/usr/bin/env python3
"""Cold-cache planner input regressions, using fake Cargo without networking."""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('cache_ci', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class CacheInputTests(unittest.TestCase):
    setUp = fixtures.ConsumerExecutionTests.setUp
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    squash_candidate = fixtures.ConsumerExecutionTests.squash_candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute
    receipt = fixtures.ConsumerExecutionTests.receipt

    def cold_cache(self):
        cargo = self.bin / 'cargo'
        source = cargo.read_text()
        source = source.replace("if args[0] == 'metadata':", '''if args[0] == 'metadata' and '--no-deps' not in args:
    snapshot = pathlib.Path(args[args.index('--manifest-path') + 1]).parent
    cache = pathlib.Path(os.environ['FAKE_CARGO_LOG'] + '.' + snapshot.name + '.cache')
    with open(os.environ['FAKE_CARGO_LOG'] + '.metadata', 'a') as stream:
        stream.write(json.dumps({'snapshot': snapshot.name, 'args': args,
                                'cwd': pathlib.Path.cwd().name,
                                'wrappers': [os.environ.get('RUSTC_WRAPPER'), os.environ.get('RUSTC_WORKSPACE_WRAPPER')]}) + '\\n')
    if '--offline' in args:
        if os.environ.get('FAIL_OFFLINE') == snapshot.name:
            raise SystemExit(101)
        if not cache.exists():
            print('error: no matching package named fixture-dependency found', file=sys.stderr)
            raise SystemExit(101)
    else:
        if os.environ.get('FAIL_PREPARE') == snapshot.name:
            print('error: failed to get fixture-dependency as a dependency', file=sys.stderr)
            raise SystemExit(101)
        cache.write_text('prepared')
if args[0] == 'metadata':''')
        cargo.write_text(source)

    def calls(self):
        return [json.loads(line) for line in Path(str(self.log) + '.metadata').read_text().splitlines()]

    def test_cold_cache_preparation_enables_pr_and_squash_merge_group_scope(self):
        for event in ('pull_request', 'merge_group'):
            with self.subTest(event=event):
                if event == 'pull_request':
                    self.candidate()
                    self.cold_cache()
                else:
                    self.git('update-ref', 'refs/pull/17/head', self.head)
                    self.git('remote', 'add', 'origin', self.repo.as_uri())
                    self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                                          '-p', self.base, '-m', 'fixture squash queue')
                    self.git('checkout', '-q', '--detach', self.merge)
                for snapshot in ('base', 'candidate'):
                    Path(str(self.log) + '.' + snapshot + '.cache').unlink(missing_ok=True)
                Path(str(self.log) + '.metadata').unlink(missing_ok=True)
                result, _ = self.execute(event, env={'NILS_CLI_MERGE_PLAN_PREPARE_METADATA': '1'})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.receipt()['validation_scope'], 'SCOPED',
                                 'cold restored cache still forced FULL instead of preparing planner inputs')
                self.assertEqual(self.receipt()['packages'], ['consumer', 'selected'])
                calls = self.calls()
                self.assertEqual([(c['snapshot'], '--offline' in c['args']) for c in calls],
                                 [('base', False), ('base', True), ('candidate', False), ('candidate', True)])
                for call in calls:
                    self.assertIn('--locked', call['args'])
                    self.assertIn('--all-features', call['args'])
                    self.assertNotIn('--filter-platform', call['args'])
                    self.assertEqual(call['cwd'], 'base')
                    self.assertEqual(call['wrappers'], ['', ''])
                prepared_digest = self.receipt()['plan_digest']
                result, _ = self.execute(event, env={'NILS_CLI_MERGE_PLAN_PREPARE_METADATA': '0'})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.receipt()['plan_digest'], prepared_digest,
                                 'preparation changed the offline plan evidence')

    def test_offline_default_retains_cold_cache_hold_and_full_fallback(self):
        self.candidate()
        self.cold_cache()
        result, rows = self.execute(env={'NILS_CLI_MERGE_PLAN_PREPARE_METADATA': '0'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'planner_hold')
        plan = json.loads((self.output / 'plan.json').read_text())
        self.assertEqual(plan['error']['stage'], 'base_metadata')
        self.assertEqual(plan['error']['recovery']['exit_code'], 101)
        self.assertEqual(len(self.calls()), 1)

    def test_preparation_failure_at_either_snapshot_retains_hold_full_and_diagnostics(self):
        self.candidate()
        self.cold_cache()
        for snapshot in ('base', 'candidate'):
            with self.subTest(snapshot=snapshot):
                self.log.unlink(missing_ok=True)
                result, rows = self.execute(env={'NILS_CLI_MERGE_PLAN_PREPARE_METADATA': '1',
                                                'FAIL_PREPARE': snapshot})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(rows, [{'kind': 'full'}])
                plan = json.loads((self.output / 'plan.json').read_text())
                self.assertEqual((plan['status'], plan['action'], plan['scope']), ('PLAN_ERROR', 'HOLD', None))
                self.assertEqual(plan['error']['stage'], snapshot + '_metadata')
                self.assertEqual(plan['error']['recovery']['exit_code'], 101)
                self.assertIn('dependency source unavailable', plan['error']['recovery']['stderr_tail'])
                self.assertEqual(self.receipt()['validation_scope'], 'FULL')

    def test_both_ci_jobs_enable_preparation_after_cache_restore(self):
        workflow = (fixtures.ROOT / '.github/workflows/ci.yml').read_text()
        for job, following in (('test', 'test_containment'), ('test_macos', 'coverage')):
            body = workflow.split(f'\n  {job}:\n', 1)[1].split(f'\n  {following}:\n', 1)[0]
            validation = body.split('      - name: Nils CLI checks ', 1)[1]
            self.assertIn('NILS_CLI_MERGE_PLAN_PREPARE_METADATA: "1"', validation)
            self.assertLess(body.index('name: Cache cargo'), body.index('name: Nils CLI checks'))

    def test_successful_preparation_cannot_replace_failed_offline_metadata(self):
        self.candidate()
        self.cold_cache()
        result, rows = self.execute(env={'NILS_CLI_MERGE_PLAN_PREPARE_METADATA': '1',
                                        'FAIL_OFFLINE': 'candidate'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['validation_scope'], 'FULL')
        plan = json.loads((self.output / 'plan.json').read_text())
        self.assertEqual(plan['error']['stage'], 'candidate_metadata')

    def test_candidate_configuration_is_held_before_online_preparation(self):
        self.candidate('hold')
        self.cold_cache()
        result, rows = self.execute(env={'NILS_CLI_MERGE_PLAN_PREPARE_METADATA': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertFalse(Path(str(self.log) + '.metadata').exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
