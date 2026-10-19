#!/usr/bin/env python3
"""Offline base/candidate coverage regressions through the merge-plan consumer."""
import importlib.util
import json
import os
import tempfile
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('ci_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class RatchetTests(unittest.TestCase):
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute
    receipt = fixtures.ConsumerExecutionTests.receipt

    def test_baseline_clone_and_target_use_canonical_temp_root(self):
        with tempfile.TemporaryDirectory(prefix='coverage-temp-root-') as canonical:
            canonical_tmp = Path(canonical)
            symlinked_tmp = canonical_tmp.parent / f'{canonical_tmp.name}-alias'
            symlinked_tmp.symlink_to(canonical_tmp, target_is_directory=True)
            self.addCleanup(symlinked_tmp.unlink, missing_ok=True)
            result, calls = self.coverage(TMPDIR=str(symlinked_tmp))

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        baseline = next(row for row in calls if row['stage'] == 'BASE')
        baseline_cwd = Path(baseline['cwd'])
        baseline_target = Path(baseline['target'])
        self.assertEqual(baseline_cwd, Path(os.path.realpath(baseline_cwd)))
        self.assertEqual(baseline_target, Path(os.path.realpath(baseline_target)))
        self.assertEqual(baseline_cwd.parent, baseline_target.parent)

    def setUp(self):
        fixtures.ConsumerExecutionTests.setUp(self)
        helper = ROOT / 'scripts/ci/scoped-coverage-ratchet.py'
        if helper.exists():
            (self.repo / 'scripts/ci/scoped-coverage-ratchet.py').write_bytes(helper.read_bytes())
        (self.bin / 'cargo').rename(self.bin / 'cargo-base')
        cargo = self.bin / 'cargo'
        cargo.write_text('''#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
args = sys.argv[1:]
if args[:2] == ['llvm-cov', 'report'] and '--exclude-from-report' in args:
    sys.stderr.write('--exclude-from-report is not supported for subcommand report\\n')
    raise SystemExit(1)
if args[:2] != ['llvm-cov', 'nextest']:
    raise SystemExit(subprocess.call([str(pathlib.Path(__file__).with_name('cargo-base')), *args]))
root = pathlib.Path.cwd()
is_head = 'changed' in (root / 'crates/selected/src/lib.rs').read_text()
stage = 'HEAD' if is_head else 'BASE'
with open(os.environ['FAKE_CARGO_LOG'], 'a') as stream:
    stream.write(json.dumps({'kind': 'cargo', 'args': args, 'stage': stage,
        'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        'cwd': str(root), 'target': os.environ.get('CARGO_TARGET_DIR'),
        'identity_present': 'FORGE_IDENTITY_FIXTURE_MARKER' in os.environ}) + '\\n')
if os.environ.get(stage + '_FAIL'):
    raise SystemExit(17)
hit = int(os.environ.get(stage + '_HIT', '80'))
found = int(os.environ.get(stage + '_FOUND', '100'))
report = pathlib.Path(args[args.index('--output-path') + 1])
if not os.environ.get(stage + '_MISSING'):
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(f'SF:{root}/crates/selected/src/lib.rs\\nLH:{hit}\\nLF:{found}\\nend_of_record\\n')
if '--fail-under-lines' in args and 100 * hit / found < float(args[args.index('--fail-under-lines') + 1]):
    raise SystemExit(1)
''')
        cargo.chmod(0o755)
        self.env.update(NILS_CLI_TEST_RUNNER='llvm-cov', NILS_CLI_SKIP_DOCTESTS='1',
                        CARGO_TARGET_DIR=str(self.root / 'candidate-target'))

    def coverage(self, **env):
        self.candidate()
        self.refs = self.git('show-ref')
        result, rows = self.execute(env=env)
        calls = [row for row in rows if row.get('args', [])[:2] == ['llvm-cov', 'nextest']]
        return result, calls

    def assert_passes(self, head):
        result, calls = self.coverage(HEAD_HIT=head)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([row['stage'] for row in calls], ['BASE', 'HEAD'])
        self.assertIn('coverage ratchet: PASS', result.stdout)
        self.assertEqual(self.receipt()['status'], 'PASS')
        self.assertEqual(self.receipt()['coverage_policy'], 'base-no-decrease')
        self.assertEqual(self.receipt()['coverage_base'], self.base)
        self.assertIsNone(self.receipt()['coverage_fail_under_lines'])

    def test_equal_coverage_below_full_floor_passes(self):
        self.assert_passes('80')

    def test_increasing_coverage_below_full_floor_passes(self):
        self.assert_passes('82')

    def test_decrease_above_absolute_floor_fails(self):
        result, calls = self.coverage(BASE_HIT='95', HEAD_HIT='94')
        self.assertNotEqual(result.returncode, 0, 'a decrease passed the old absolute floor')
        self.assertEqual([row['stage'] for row in calls], ['BASE', 'HEAD'])
        self.assertIn('coverage ratchet: FAIL', result.stdout)
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_same_corpus_and_bound_base_use_isolated_profile_directories(self):
        result, calls = self.coverage(NILS_CLI_SCOPED_COVERAGE_BASE='0' * 40)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual([row['revision'] for row in calls], [self.base, self.merge])
        self.assertEqual(len(calls), 2)
        for row in calls:
            args = row['args']
            self.assertNotIn('--workspace', args)
            self.assertNotIn('--fail-under-lines', args)
            self.assertIn('--all-features', args)
            self.assertEqual([args[i + 1] for i, arg in enumerate(args) if arg == '-p'], ['consumer', 'selected'])
            self.assertEqual([args[i + 1] for i, arg in enumerate(args) if arg == '--exclude-from-report'], ['unrelated', 'untouched'])
            self.assertFalse(row['identity_present'])
        self.assertNotEqual(calls[0]['target'], calls[1]['target'])
        self.assertFalse(Path(calls[0]['cwd']).exists(), 'temporary baseline clone leaked')
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.merge)
        self.assertEqual(self.git('show-ref'), self.refs)
        self.assertEqual(self.git('diff', '--name-only'), '')
        self.assertEqual(self.git('diff', '--cached', '--name-only'), '')
        evidence = self.repo / 'target/ci-merge-plan/scoped-coverage'
        proof = json.loads((evidence / 'ratchet.json').read_text())
        self.assertEqual(proof['base'], self.base)
        self.assertEqual(proof['candidate'], self.merge)
        self.assertEqual(proof['status'], 'PASS')
        self.assertTrue((evidence / 'base-lcov.info').is_file())

    def test_baseline_failure_does_not_run_candidate_coverage(self):
        result, calls = self.coverage(BASE_FAIL='1')
        self.assertEqual(result.returncode, 17, result.stdout + result.stderr)
        self.assertEqual([row['stage'] for row in calls], ['BASE'])
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_missing_baseline_report_fails_closed(self):
        result, _ = self.coverage(BASE_MISSING='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('coverage ratchet:', result.stderr)
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_stale_baseline_report_cannot_substitute_missing_output(self):
        self.candidate()
        stale = self.repo / 'target/coverage/base-lcov.info'
        stale.parent.mkdir(parents=True)
        stale.write_text('SF:fixture\nLH:80\nLF:100\nend_of_record\n')
        result, _ = self.execute(env={'BASE_MISSING': '1'})
        self.assertNotEqual(result.returncode, 0, 'stale baseline evidence passed')
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_candidate_failure_is_not_a_ratchet_pass(self):
        result, calls = self.coverage(HEAD_FAIL='1')
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual([row['stage'] for row in calls], ['BASE', 'HEAD'])
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_fraction_comparison_rejects_decrease_hidden_by_display_rounding(self):
        result, _ = self.coverage(BASE_HIT='800000000', BASE_FOUND='1000000000',
                                  HEAD_HIT='799999999', HEAD_FOUND='1000000000')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('coverage ratchet: FAIL', result.stdout)

    def test_empty_baseline_corpus_fails_closed(self):
        result, _ = self.coverage(BASE_HIT='0', BASE_FOUND='0')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('empty or incomplete LCOV corpus', result.stderr)

    def test_missing_candidate_report_fails_closed(self):
        result, _ = self.coverage(HEAD_MISSING='1')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('coverage ratchet:', result.stderr)

    def test_invalid_lcov_counts_fail_closed(self):
        result, _ = self.coverage(BASE_HIT='101', BASE_FOUND='100')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('invalid LCOV record', result.stderr)

    def test_html_report_keeps_selected_corpus_without_second_absolute_floor(self):
        self.candidate()
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        report = next(row['args'] for row in rows if row.get('args', [])[:2] == ['llvm-cov', 'report'])
        self.assertNotIn('--fail-under-lines', report)
        self.assertNotIn('--exclude-from-report', report)
        self.assertEqual([report[i + 1] for i, arg in enumerate(report) if arg == '-p'],
                         ['consumer', 'selected'])

    def test_full_workspace_absolute_floor_is_retained(self):
        for relative in ['scripts/ci/nils-cli-checks-entrypoint.sh',
                         '.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh']:
            text = (ROOT / relative).read_text()
            self.assertIn('--fail-under-lines', text)
            self.assertIn(':-85', text)


if __name__ == '__main__':
    unittest.main(verbosity=2)
