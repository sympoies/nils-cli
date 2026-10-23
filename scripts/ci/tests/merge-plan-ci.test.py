#!/usr/bin/env python3
"""Offline CI dispatch regressions: never launch a compiler or hosted job."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
CONSUMER = ROOT / 'scripts/ci/nils-cli-merge-plan-checks.py'
spec = importlib.util.spec_from_file_location('ci_consumer', CONSUMER)
consumer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(consumer)


def seal(plan):
    plan['digests'].pop('plan', None)
    plan['digests']['plan'] = hashlib.sha256(json.dumps(
        plan, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()).hexdigest()
    return plan


def checks_script(job):
    workflow = (ROOT / '.github/workflows/ci.yml').read_text()
    body = workflow.split(f'\n  {job}:\n', 1)[1]
    body = re.split(r'\n  [a-z_]+:\n', body, maxsplit=1)[0]
    step = body.split('      - name: Nils CLI checks ', 1)[1]
    script = step.split('        run: |\n', 1)[1]
    script = re.split(r'\n      - name:', script, maxsplit=1)[0]
    return '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))


class WorkflowDispatchTests(unittest.TestCase):
    def dispatch(self, job, event):
        with tempfile.TemporaryDirectory(prefix='ci-plan-event-') as directory:
            root = Path(directory)
            entrypoint = root / 'scripts/ci/nils-cli-checks-entrypoint.sh'
            entrypoint.parent.mkdir(parents=True)
            entrypoint.write_text('#!/usr/bin/env bash\nset -euo pipefail\nprintf "%s\\n" "$@" > "$CALL_LOG"\n')
            payload = root / 'event.json'
            payload.write_text(json.dumps({'fixture': event}))
            script = checks_script(job)
            script = script.replace('/opt/homebrew/bin/bash', 'bash')
            script = script.replace('${{ needs.changes.outputs.docs_only }}', 'false')
            script = script.replace('${{ needs.changes.outputs.release_only }}', 'false')
            env = dict(os.environ, GITHUB_EVENT_NAME=event, GITHUB_EVENT_PATH=str(payload),
                       GITHUB_SHA='a' * 40, CALL_LOG=str(root / 'call.log'))
            result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], cwd=root,
                                    env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return (root / 'call.log').read_text().splitlines()

    def test_pr_and_queue_dispatch_plan_consumption(self):
        for event in ('pull_request', 'merge_group'):
            self.assertIn('--ci-merge-plan', self.dispatch('test', event))

    def test_scoped_checks_cannot_claim_full_release_provenance(self):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        markers = re.findall(r'      - name: Full validation marker\n        if: ([^\n]+)', workflow)
        self.assertEqual(len(markers), 1)
        for scope in ('SCOPED', 'FULL'):
            expression = markers[0]
            for key, value in {'needs.changes.outputs.docs_only': 'false',
                    'needs.changes.outputs.release_only': 'false',
                    'steps.validation.outputs.validation_scope': scope}.items():
                expression = expression.replace(key, repr(value))
            self.assertEqual(eval(expression.replace('&&', 'and'), {'__builtins__': {}}, {}), scope == 'FULL')


class ReleaseOnlyLaneTests(unittest.TestCase):
    """A canonical release bump takes the reduced lane without a base-CI proof.

    The release workflow validates the tagged commit in full, and scoped queue
    runs on the base carry no full-validation marker to prove.
    """
    def finalize(self, candidate, base_ci=None):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        step = workflow.split('      - name: Finalize fail-closed CI lane\n', 1)[1]
        script = step.split('        run: |\n', 1)[1]
        script = re.split(r'\n  [a-z_]+:\n', script, maxsplit=1)[0]
        script = '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))
        with tempfile.TemporaryDirectory(prefix='ci-finalize-') as directory:
            output = Path(directory) / 'output'
            env = dict(os.environ, GITHUB_OUTPUT=str(output), RELEASE_CANDIDATE=candidate)
            env.pop('BASE_CI_TRUSTED', None)
            if base_ci is not None:
                env['BASE_CI_TRUSTED'] = base_ci
            subprocess.run(['bash', '-euo', 'pipefail', '-c', script], env=env, check=True,
                           capture_output=True)
            return output.read_text().strip()

    def test_release_candidate_alone_selects_the_reduced_lane(self):
        for base_ci in (None, 'false', 'true'):
            with self.subTest(base_ci=base_ci):
                self.assertEqual(self.finalize('true', base_ci), 'release_only=true')

    def test_non_candidates_never_take_the_reduced_lane(self):
        for base_ci in (None, 'true'):
            with self.subTest(base_ci=base_ci):
                self.assertEqual(self.finalize('false', base_ci), 'release_only=false')

    def test_workflow_no_longer_waits_for_a_base_ci_proof(self):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        self.assertNotIn('Prove full CI on exact base main commit', workflow)
        self.assertNotIn('steps.base_ci', workflow)


class ConsumerExecutionTests(unittest.TestCase):
    """Real Git/planner and fake Cargo executor; every command stays offline."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ci-plan-execution-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repository'
        self.repo.mkdir()
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.log = self.root / 'commands.jsonl'
        self.audit_log = self.root / 'audits.log'
        self.output = self.root / 'receipt'
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ['PATH'],
                        FAKE_CARGO_LOG=str(self.log), FAKE_AUDIT_LOG=str(self.audit_log),
                        NILS_CLI_TEST_RUNNER='nextest',
                        NILS_CLI_SKIP_DOCTESTS='0', NILS_CLI_COVERAGE_FAIL_UNDER_LINES='85',
                        GITHUB_OUTPUT=str(self.root / 'github-output'),
                        NILS_CLI_REPORT_DIR=str(self.root / 'reports'),
                        FORGE_IDENTITY_FIXTURE_MARKER='synthetic')
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        paths = ('scripts/ci/nils-cli-merge-plan.py', 'scripts/ci/lib/merge_plan.py',
                 'scripts/ci/merge-plan-policy.v1.json', 'scripts/ci/merge-plan-extra-inputs.v1.json',
                 'scripts/ci/merge-plan.schema.v1.json', 'scripts/ci/merge-plan.schema.v2.json',
                 'scripts/ci/nils-cli-scoped-checks.sh', 'scripts/ci/scoped-coverage-ratchet.py',
                 'scripts/ci/assert-import-audit.py', 'scripts/ci/lib/rust_file_size.py',
                 'scripts/ci/lib/sibling_bins.py',
                 'scripts/ci/test-env.sh', 'scripts/ci/gate-reports.py')
        for path in paths:
            dest = self.repo / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes((ROOT / path).read_bytes())
        (self.repo / 'Cargo.toml').write_text('[workspace]\nmembers = []\n')
        audit_test = self.repo / 'scripts/ci/tests/assert-import-audit.test.py'
        audit_test.parent.mkdir(parents=True)
        audit_test.write_text('# Audit self-tests run separately against the complete source tree.\n')
        for name in ('selected', 'consumer', 'untouched', 'unrelated'):
            source = self.repo / f'crates/{name}/src/lib.rs'
            source.parent.mkdir(parents=True)
            source.write_text('// synthetic fixture\n')
        # Audits are recorded, not run; FAIL_AUDIT names the one that must fail.
        for name in ('docs-placement-audit', 'docs-hygiene-audit', 'docs-prose-audit',
                     'file-size-audit', 'tempdir-leak-audit', 'markdownlint-audit',
                     'cli-output-contract-lint', 'forge-cli-fixture-lint', 'test-stale-audit',
                     'completion-asset-audit', 'completion-freshness-audit',
                     'completion-flag-parity-audit'):
            (self.repo / f'scripts/ci/{name}.sh').write_text(
                '#!/usr/bin/env bash\n'
                'echo "$(basename "$0" .sh) $*" >> "$FAKE_AUDIT_LOG"\n'
                '[[ "$(basename "$0" .sh)" != "${FAIL_AUDIT:-}" ]]\n')
        (self.repo / 'tests/zsh').mkdir(parents=True)
        (self.repo / 'tests/zsh/completion.test.zsh').write_text('# synthetic completion suite\n')
        zsh = self.bin / 'zsh'
        zsh.write_text('#!/usr/bin/env bash\necho "zsh $*" >> "$FAKE_AUDIT_LOG"\n'
                       '[[ "${FAIL_AUDIT:-}" != zsh-completion ]]\n')
        zsh.chmod(0o755)
        (self.repo / 'scripts/ci/nils-cli-checks-entrypoint.sh').write_text(
            '#!/usr/bin/env bash\nprintf \'{"kind":"full"}\\n\' >> "$FAKE_CARGO_LOG"\n')
        cargo = self.bin / 'cargo'
        cargo.write_text('''#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
if args[0] == 'metadata':
    if '--no-deps' not in args and os.environ.get('FAIL_PLAN_METADATA'):
        raise SystemExit(47)
    if '--no-deps' in args and os.environ.get('FAIL_REPORT_METADATA'):
        raise SystemExit(43)
    root = pathlib.Path(args[args.index('--manifest-path') + 1]).parent if '--manifest-path' in args else pathlib.Path.cwd()
    names = ('selected', 'consumer', 'untouched', 'unrelated')
    ids = {n: f'path+{root.as_uri()}/crates/{n}#{n}@0.1.0' for n in names}
    bins = [{'kind': ['bin'], 'name': os.environ['FIXTURE_SELECTED_BIN']}] if os.environ.get('FIXTURE_SELECTED_BIN') else []
    sibling = [{'kind': ['bin'], 'name': 'sibling-cli'}] if os.environ.get('FIXTURE_SIBLING_BIN') else []
    libs = {n: [{'kind': ['lib'], 'crate_types': ['lib'], 'name': n}] for n in names}
    if os.environ.get('FIXTURE_SELECTED_BIN_ONLY'):
        libs['selected'] = []
    if os.environ.get('FIXTURE_ALL_BIN_ONLY'):
        libs['selected'] = []
        libs['consumer'] = []
    packages = []
    for n in names:
        targets = libs[n]
        if n == 'selected' and (os.environ.get('FIXTURE_SELECTED_BIN_ONLY')
                                or os.environ.get('FIXTURE_ALL_BIN_ONLY')):
            targets = targets + (bins or [{'kind': ['bin'], 'name': 'selected-cli'}])
        elif n == 'consumer' and os.environ.get('FIXTURE_ALL_BIN_ONLY'):
            targets = targets + [{'kind': ['bin'], 'name': 'consumer-cli'}]
        elif n == 'selected':
            targets = targets + bins
        elif n == 'unrelated':
            targets = targets + sibling
        packages.append({'id': ids[n], 'name': n, 'manifest_path': str(root/f'crates/{n}/Cargo.toml'),
                         'targets': targets})
    nodes = [{'id': ids[n], 'deps': [{'pkg': ids['selected']}] if n == 'consumer' else []} for n in names]
    print(json.dumps({'workspace_root': str(root), 'workspace_members': list(ids.values()), 'packages': packages, 'resolve': {'nodes': nodes}}))
else:
    with open(os.environ['FAKE_CARGO_LOG'], 'a') as stream:
        stream.write(json.dumps({'kind': 'cargo', 'args': args, 'identity_present': 'FORGE_IDENTITY_FIXTURE_MARKER' in os.environ}) + '\\n')
    if args[0] == os.environ.get('FAIL_CARGO_COMMAND'):
        raise SystemExit(17)
''')
        cargo.chmod(0o755)
        rustc = self.bin / 'rustc'
        rustc.write_text('#!/usr/bin/env bash\necho compiler-invoked > "$FAKE_CARGO_LOG.compiler"\nexit 99\n')
        rustc.chmod(0o755)

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args], stderr=subprocess.DEVNULL).decode().strip()

    def candidate(self, change='source', missing_launcher=False):
        if missing_launcher:
            (self.repo / 'scripts/ci/nils-cli-merge-plan.py').unlink()
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture base')
        self.base = self.git('rev-parse', 'HEAD')
        if change == 'source':
            (self.repo / 'crates/selected/src/lib.rs').write_text('// changed synthetic fixture\n')
        elif change == 'ambiguous':
            (self.repo / 'crates/selected/src/lib.rs').write_text(
                '#[cfg(test)] mod tests { use pretty_assertions::assert_eq; '
                'mod child { use super::*; fn check() { assert_eq!(1, 1); } } }\n')
        elif change == 'bulk':
            for index in range(30):
                (self.repo / f'crates/selected/src/bulk_{index}.rs').write_text('// bulk\n')
            (self.repo / 'crates/selected/src/`x|y`.rs').write_text('// hostile\n')
        elif change == 'completion':
            (self.repo / 'completions/zsh/_selected-cli').write_text('# regenerated asset\n')
        elif change == 'unknown-completion':
            (self.repo / 'completions/zsh/_unknown-cli').write_text('# regenerated asset\n')
        elif change == 'full':
            with (self.repo / 'Cargo.toml').open('a') as stream:
                stream.write('# manifest escalation\n')
        elif change == 'hold':
            (self.repo / 'rust-toolchain.toml').write_text('[toolchain]\nchannel="fixture"\n')
        elif change == 'selector':
            (self.repo / 'scripts/ci/nils-cli-merge-plan.py').write_text(
                'from pathlib import Path\nPath("candidate-launcher-ran").write_text("bad")\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture head')
        self.head = self.git('rev-parse', 'HEAD')
        tree = self.git('rev-parse', 'HEAD^{tree}')
        self.merge = self.git('commit-tree', tree, '-p', self.base, '-p', self.head, '-m', 'fixture candidate')
        self.git('checkout', '-q', '--detach', self.merge)

    def event(self, name):
        if name == 'pull_request':
            return {'pull_request': {'base': {'sha': self.base}, 'head': {'sha': self.head}}}
        return {'merge_group': {'base_sha': self.base, 'head_sha': self.merge,
                                'head_ref': f'refs/heads/gh-readonly-queue/main/pr-17-{self.base}'}}

    def squash_candidate(self):
        self.candidate()
        self.git('update-ref', 'refs/pull/17/head', self.head)
        self.git('remote', 'add', 'origin', self.repo.as_uri())
        self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                              '-p', self.base, '-m', 'fixture squash queue')
        self.git('checkout', '-q', '--detach', self.merge)

    def execute(self, name='pull_request', event=None, env=None, extra_args=()):
        payload = self.root / 'event.json'
        payload.write_text(json.dumps(self.event(name) if event is None else event))
        result = subprocess.run([sys.executable, str(CONSUMER), '--repo', str(self.repo),
                                 '--event-name', name, '--event-path', str(payload),
                                 '--candidate', self.merge, '--output', str(self.output), *extra_args],
                                env=dict(self.env, **(env or {})), capture_output=True, text=True)
        self.assertFalse(Path(str(self.log) + '.compiler').exists(), 'real compiler sentinel ran')
        rows = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, rows

    def receipt(self):
        return json.loads((self.output / 'validation.json').read_text())

    def test_pr_executes_exact_selected_clippy_binaries_tests_and_doctests(self):
        self.candidate()
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = self.receipt()
        self.assertEqual(receipt['validation_scope'], 'SCOPED')
        self.assertEqual(receipt['packages'], ['consumer', 'selected'])
        self.assertEqual(receipt['inputs']['head'], self.head)
        calls = [r['args'] for r in rows]
        for prefix in ('clippy', 'build', 'nextest', 'test'):
            command = next(c for c in calls if c[0] == prefix)
            self.assertNotIn('--workspace', command)
            self.assertEqual([command[i + 1] for i, arg in enumerate(command) if arg == '-p'],
                             ['consumer', 'selected'])
        self.assertEqual(next(c for c in calls if c[0] == 'clippy')[-2:], ['-D', 'warnings'])
        self.assertTrue(all(not r['identity_present'] for r in rows if r['args'][0] in ('nextest', 'test')))

    def test_bin_only_selected_package_skips_doctests(self):
        self.selected_binary()
        self.candidate()
        result, rows = self.execute(env={'FIXTURE_SELECTED_BIN_ONLY': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.receipt()['packages'], ['consumer', 'selected'])
        doctest = next(row['args'] for row in rows if row['args'][0] == 'test')
        self.assertEqual([doctest[index + 1] for index, arg in enumerate(doctest) if arg == '-p'],
                         ['consumer'])

    def test_all_bin_only_selected_packages_skip_doctests(self):
        self.selected_binary()
        self.candidate()
        result, rows = self.execute(env={'FIXTURE_ALL_BIN_ONLY': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.receipt()['packages'], ['consumer', 'selected'])
        self.assertFalse(any(row['args'][0] == 'test' for row in rows))

    def selected_binary(self):
        """Give `selected` a required `selected-cli` binary and its generated asset."""
        matrix = self.repo / 'docs/specs/completion-coverage-matrix-v1.md'
        matrix.parent.mkdir(parents=True, exist_ok=True)
        matrix.write_text('| Binary | Obligation |\n| --- | --- |\n| `selected-cli` | `required` |\n')
        for path in ('completions/zsh/_selected-cli', 'completions/zsh/_unknown-cli'):
            (self.repo / path).parent.mkdir(parents=True, exist_ok=True)
            (self.repo / path).write_text('# synthetic completion asset\n')
        self.env['FIXTURE_SELECTED_BIN'] = 'selected-cli'

    def audits(self):
        return self.audit_log.read_text().splitlines() if self.audit_log.exists() else []

    def test_scoped_lane_runs_the_cheap_audits_a_crate_change_can_break(self):
        self.candidate()
        result, _ = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        audits = self.audits()
        for expected in ('markdownlint-audit --strict', 'cli-output-contract-lint --strict',
                         'forge-cli-fixture-lint --strict', 'test-stale-audit --strict',
                         'completion-asset-audit --strict', 'zsh -f tests/zsh/completion.test.zsh'):
            with self.subTest(audit=expected):
                self.assertIn(expected, audits)

    def test_scoped_flag_parity_audits_selected_binaries_without_a_workspace_build(self):
        self.selected_binary()
        self.candidate()
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('completion-flag-parity-audit --strict --skip-build --bin selected-cli', self.audits())
        builds = [r['args'] for r in rows if r['args'][0] == 'build']
        self.assertEqual(len(builds), 1)
        self.assertNotIn('--workspace', builds[0])

    def test_scoped_lane_builds_but_does_not_test_sibling_binaries_the_tests_resolve(self):
        # forge-cli tests run the semantic-commit binary: a package-scoped run
        # builds only the selected packages' own binaries (#2711 regression).
        (self.repo / 'crates/selected/src/sibling.rs').write_text(
            '// nils_test_support::bin::sibling_or_skip("sibling-cli", "unrelated")\n')
        self.env['FIXTURE_SIBLING_BIN'] = '1'
        self.candidate()
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.receipt()['packages'], ['consumer', 'selected'])

        def packages(prefix):
            command = next(r['args'] for r in rows if r['args'][0] == prefix)
            return [command[i + 1] for i, arg in enumerate(command) if arg == '-p']

        self.assertEqual(packages('build'), ['consumer', 'selected', 'unrelated'])
        for prefix in ('clippy', 'nextest', 'test'):
            self.assertEqual(packages(prefix), ['consumer', 'selected'])

    def test_scoped_lane_fails_before_compiling_when_a_cheap_audit_fails(self):
        self.candidate()
        for name in ('markdownlint-audit', 'cli-output-contract-lint', 'forge-cli-fixture-lint',
                     'test-stale-audit', 'completion-asset-audit', 'zsh-completion'):
            with self.subTest(audit=name):
                self.log.unlink(missing_ok=True)
                result, rows = self.execute(env={'FAIL_AUDIT': name})
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual([r['args'][0] for r in rows if r['args'][0] in ('clippy', 'build')], [])
                self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_generated_completion_asset_alone_runs_scoped_for_its_owner(self):
        self.selected_binary()
        self.candidate('completion')
        result, _ = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.receipt()['validation_scope'], 'SCOPED')
        self.assertEqual(self.receipt()['packages'], ['consumer', 'selected'])

    def test_completion_asset_without_a_binary_owner_runs_full(self):
        self.selected_binary()
        self.candidate('unknown-completion')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['validation_scope'], 'FULL')

    def summary(self, name='pull_request', change='source', env=None):
        self.candidate(change)
        path = self.root / 'step-summary.md'
        result, _ = self.execute(name, env=dict(env or {}, GITHUB_STEP_SUMMARY=str(path)))
        self.assertEqual(result.returncode, 0, result.stderr)
        return path.read_text()

    def test_job_summary_shows_scope_packages_and_reasons_for_pr_and_queue(self):
        for name in ('pull_request', 'merge_group'):
            with self.subTest(event=name):
                self.setUp()
                text = self.summary(name)
                self.assertIn('SCOPED', text)
                self.assertIn('`consumer`', text)
                self.assertIn('`selected`', text)
                self.assertIn('CHANGED_PACKAGE', text)
                self.assertIn('REVERSE_DEPENDENCY', text)
                self.assertIn('crates/selected/src/lib.rs', text)

    def test_job_summary_explains_a_full_escalation(self):
        text = self.summary(change='full')
        self.assertIn('FULL', text)
        self.assertIn('ALWAYS_ESCALATE', text)
        self.assertIn('`Cargo.toml`', text)

    def test_job_summary_names_the_fallback_when_there_is_no_usable_plan(self):
        text = self.summary(change='hold')
        self.assertIn('FULL', text)
        self.assertIn('planner_hold', text)

    def test_job_summary_keeps_untrusted_paths_inert_and_bounded(self):
        text = self.summary(change='bulk')
        self.assertNotIn('x|y', text)
        self.assertLess(text.count('bulk_'), 10)
        self.assertIn('CHANGED_PACKAGE', text)

    def test_ambiguous_assert_imports_fail_before_compilation(self):
        self.candidate(change='ambiguous')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(rows, [])
        self.assertIn('explicitly import pretty_assertions::assert_eq', result.stdout)
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_merge_group_head_is_candidate_not_reviewed_head(self):
        self.candidate()
        result, _ = self.execute('merge_group')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotEqual(self.head, self.merge)
        self.assertEqual(self.receipt()['inputs'], {'base': self.base, 'head': self.head,
                                                   'candidate': self.merge,
                                                   'tree': self.git('rev-parse', 'HEAD^{tree}')})

    def test_squash_queue_adapts_exact_tree_to_accepted_plan(self):
        self.squash_candidate()
        refs = self.git('show-ref')
        result, _ = self.execute('merge_group')
        self.assertEqual(result.returncode, 0, result.stderr)
        receipt = self.receipt()
        self.assertEqual(receipt['validation_scope'], 'SCOPED')
        self.assertEqual(receipt['inputs']['head'], self.head)
        self.assertNotEqual(receipt['inputs']['candidate'], self.merge)
        self.assertEqual(self.git('show', '-s', '--format=%P', receipt['inputs']['candidate']).split(),
                         [self.base, self.head])
        self.assertEqual(receipt['checkout'], {'commit': self.merge,
                                              'tree': self.git('rev-parse', 'HEAD^{tree}')})
        self.assertEqual(receipt['inputs']['tree'], receipt['checkout']['tree'])
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.merge)
        self.assertEqual(self.git('show-ref'), refs)

    def test_squash_queue_changed_source_tree_requires_full_fallback(self):
        self.squash_candidate()
        self.git('update-ref', 'refs/pull/17/head', self.base)
        result, rows = self.execute('merge_group')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'squash_tree_mismatch')
        self.assertIsNone(self.receipt()['plan_digest'])

    def test_squash_queue_unknown_provenance_uses_full_not_scoped_credit(self):
        self.squash_candidate()
        event = self.event('merge_group')
        event['merge_group']['head_ref'] = 'refs/heads/ordinary-branch'
        result, rows = self.execute('merge_group', event)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'squash_provenance_unavailable')

    def test_squash_queue_unavailable_source_uses_full_without_network_retry(self):
        self.squash_candidate()
        self.git('remote', 'set-url', 'origin', (self.root / 'missing-remote').as_uri())
        result, rows = self.execute('merge_group')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'squash_provenance_unavailable')

    def test_squash_proof_never_executes_configured_merge_driver(self):
        self.squash_candidate()
        marker = self.root / 'driver-ran'
        self.git('config', 'merge.fixture.driver', f'touch {marker}')
        result, rows = self.execute('merge_group')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'squash_merge_unsupported')
        self.assertFalse(marker.exists())

    def test_squash_proof_attribute_paths_use_portable_nul_framing(self):
        self.squash_candidate()
        self.git('checkout', '-q', '--detach', self.head)
        attribute = self.repo / 'nested\nfixture/.GITATTRIBUTES'
        attribute.parent.mkdir()
        attribute.write_text('* merge=fixture\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture attributes')
        source = self.git('rev-parse', 'HEAD')
        self.git('update-ref', 'refs/pull/17/head', source)
        self.merge = self.git('commit-tree', self.git('rev-parse', 'HEAD^{tree}'),
                              '-p', self.base, '-m', 'fixture squash attributes')
        self.git('checkout', '-q', '--detach', self.merge)
        result, rows = self.execute('merge_group')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'squash_merge_unsupported')

    def test_full_escalation_executes_existing_full_gate(self):
        self.candidate('full')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['validation_scope'], 'FULL')
        self.assertEqual(json.loads((self.output / 'plan.json').read_text())['scope'], 'FULL')

    def test_planner_hold_keeps_error_and_runs_full_fallback(self):
        self.candidate('hold')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'planner_hold')
        plan = json.loads((self.output / 'plan.json').read_text())
        self.assertEqual((plan['status'], plan['action'], plan['scope']), ('PLAN_ERROR', 'HOLD', None))

    def test_offline_metadata_failure_retains_hold_and_full_validation(self):
        self.candidate()
        result, rows = self.execute(env={'FAIL_PLAN_METADATA': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        plan = json.loads((self.output / 'plan.json').read_text())
        self.assertEqual(plan['error']['code'], 'METADATA_FAILED')
        self.assertEqual(self.receipt()['fallback'], 'planner_hold')

    def test_candidate_launcher_is_not_used_for_selection(self):
        self.candidate('selector')
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertFalse((self.repo / 'candidate-launcher-ran').exists())
        self.assertEqual(json.loads((self.output / 'plan.json').read_text())['scope'], 'FULL')

    def test_base_without_launcher_uses_full_without_inventing_ready_plan(self):
        self.candidate(missing_launcher=True)
        result, rows = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'full'}])
        self.assertEqual(self.receipt()['fallback'], 'trusted_base_launcher_missing')
        self.assertIsNone(self.receipt()['plan_digest'])
        self.assertFalse((self.output / 'plan.json').exists())

    def test_pr_binding_mismatch_holds_before_validation(self):
        self.candidate()
        for field in ('base', 'head'):
            with self.subTest(field=field):
                event = self.event('pull_request')
                event['pull_request'][field]['sha'] = self.merge
                result, rows = self.execute(event=event)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(rows, [])

    def test_merge_group_event_mismatch_holds_before_validation(self):
        self.candidate()
        event = self.event('merge_group')
        event['merge_group']['head_sha'] = self.head
        result, rows = self.execute('merge_group', event)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])

    def test_dirty_candidate_holds_before_validation(self):
        self.candidate()
        (self.repo / 'crates/selected/src/lib.rs').write_text('// unbound source\n')
        result, rows = self.execute()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(rows, [])

    def test_failed_selected_check_cannot_publish_pass_scope(self):
        self.candidate()
        result, _ = self.execute(env={'FAIL_CARGO_COMMAND': 'nextest'})
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(self.receipt()['status'], 'FAIL')
        self.assertFalse((self.root / 'github-output').exists())

    def test_missing_coverage_metadata_cannot_become_pass(self):
        self.candidate()
        result, rows = self.execute(env={'NILS_CLI_TEST_RUNNER': 'llvm-cov', 'FAIL_REPORT_METADATA': '1'})
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(r['args'][0] == 'llvm-cov' for r in rows))
        self.assertEqual(self.receipt()['status'], 'FAIL')

    def test_plan_digest_inputs_and_package_injection_are_rejected(self):
        self.candidate()
        expected = {'base': self.base, 'head': self.head, 'candidate': self.merge,
                    'tree': self.git('rev-parse', 'HEAD^{tree}')}
        with mock.patch.dict(os.environ, self.env):
            plan, _ = consumer.make_plan(self.repo, expected)
        for mutation in ('digest', 'inputs', 'package'):
            with self.subTest(mutation=mutation):
                changed = json.loads(json.dumps(plan))
                if mutation == 'digest':
                    changed['digests']['plan'] = '0' * 64
                elif mutation == 'inputs':
                    changed['inputs']['head'] = self.merge
                    seal(changed)
                else:
                    changed['selected_packages'][0]['name'] = '--workspace'
                    seal(changed)
                with self.assertRaises(ValueError):
                    consumer.plan_scope(changed, expected)

    def test_empty_selection_requires_full_validation_not_no_op(self):
        self.candidate()
        expected = {'base': self.base, 'head': self.head, 'candidate': self.merge,
                    'tree': self.git('rev-parse', 'HEAD^{tree}')}
        with mock.patch.dict(os.environ, self.env):
            plan, _ = consumer.make_plan(self.repo, expected)
            plan['selected_packages'] = []
            seal(plan)
            with mock.patch.object(consumer, 'make_plan', return_value=(plan, None)):
                code = consumer.run(self.repo, 'pull_request', self.event('pull_request'), self.merge, self.output)
        self.assertEqual(code, 0)
        self.assertEqual(self.receipt()['validation_scope'], 'FULL')
        self.assertEqual(self.receipt()['fallback'], 'empty_selection')

    def canonical_fixture(self):
        for path in ('scripts/ci/nils-cli-checks-entrypoint.sh',
                     'scripts/ci/nils-cli-merge-plan-checks.py'):
            (self.repo / path).write_bytes((ROOT / path).read_bytes())
        verify = self.repo / '.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh'
        verify.parent.mkdir(parents=True)
        verify.write_text('#!/usr/bin/env bash\nprintf \'{"kind":"full"}\\n\' >> "$FAKE_CARGO_LOG"\n')
        # Admission is a fixture, not a local gate: no compiler or live slot is
        # launched. The owning matrix itself runs in the caller's 6 GiB scope.
        (self.repo / 'scripts/ci/gate-resources.py').write_text('''import json, os, subprocess, sys
with open(os.environ['FAKE_CARGO_LOG'], 'a') as stream:
    stream.write(json.dumps({'kind': 'admission'}) + '\\n')
env = dict(os.environ, NILS_CLI_GATE_ACTIVE='1')
raise SystemExit(subprocess.call(sys.argv[sys.argv.index('--') + 1:], env=env))
''')
        xvfb = self.bin / 'xvfb-run'
        xvfb.write_text('#!/usr/bin/env bash\nprintf \'{"kind":"xvfb"}\\n\' >> "$FAKE_CARGO_LOG"\nshift\nexec "$@"\n')
        xvfb.chmod(0o755)
        self.env.update(NILS_CLI_GATE_ACTIVE='0')

    def run_canonical(self):
        payload = self.root / 'event.json'
        payload.write_text(json.dumps(self.event('pull_request')))
        env = dict(self.env, GITHUB_EVENT_NAME='pull_request', GITHUB_EVENT_PATH=str(payload),
                   GITHUB_SHA=self.merge)
        result = subprocess.run(['bash', 'scripts/ci/nils-cli-checks-entrypoint.sh',
                                 '--xvfb', '--ci-merge-plan'], cwd=self.repo, env=env,
                                text=True, capture_output=True)
        rows = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        self.assertFalse(Path(str(self.log) + '.compiler').exists())
        return result, rows

    def test_canonical_entrypoint_uses_own_admission_and_selected_executor(self):
        self.canonical_fixture()
        self.candidate()
        result, rows = self.run_canonical()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(row['kind'] == 'admission' for row in rows), 1)
        self.assertEqual(sum(row['kind'] == 'xvfb' for row in rows), 1)
        self.assertFalse(any(row['kind'] == 'full' for row in rows))
        self.assertIn('validation_scope=SCOPED', (self.root / 'github-output').read_text())

    def test_canonical_full_fallback_does_not_reenter_plan_or_admission(self):
        self.canonical_fixture()
        self.candidate('full')
        result, rows = self.run_canonical()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(rows, [{'kind': 'admission'}, {'kind': 'xvfb'}, {'kind': 'full'}])
        self.assertIn('validation_scope=FULL', (self.root / 'github-output').read_text())


def load_tests(loader, tests, pattern):
    spec = importlib.util.spec_from_file_location('queue_detection', Path(__file__).with_name('merge-plan-queue-detection.test.py'))
    queue = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(queue)
    tests.addTests(loader.loadTestsFromTestCase(queue.QueueDetectionTests))
    spec = importlib.util.spec_from_file_location('preview', Path(__file__).with_name('merge-plan-preview.test.py'))
    preview = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(preview)
    tests.addTests(loader.loadTestsFromTestCase(preview.PreviewTests))
    return tests


if __name__ == '__main__':
    unittest.main()
