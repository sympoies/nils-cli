#!/usr/bin/env python3
"""Execute queue change detection with real docs and release classifiers."""
import importlib.util
from pathlib import Path
import re
import subprocess
import unittest

spec = importlib.util.spec_from_file_location('queue_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


def detect_script():
    workflow = (fixtures.ROOT / '.github/workflows/ci.yml').read_text()
    step = workflow.split('      - name: Detect change set\n', 1)[1]
    script = step.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
    return '\n'.join(line[10:] for line in script.splitlines())


class QueueDetectionTests(unittest.TestCase):
    git = fixtures.ConsumerExecutionTests.git

    def setUp(self):
        fixtures.ConsumerExecutionTests.setUp(self)
        # Reuse the canonical release fixture's manifests, lockfile, checksums
        # and bump producer without executing its separate test suite.
        source = (fixtures.ROOT / 'scripts/ci/tests/detect-release-only.test.sh').read_text()
        self.helpers = 'sha256_file() {' + source.split('sha256_file() {', 1)[1].split('\ndetect() {', 1)[0]
        self.env['FIXTURE_REPO'] = str(self.repo)
        self.shell(self.helpers + '\nwrite_base_fixture "$FIXTURE_REPO"\n')
        for path in ('scripts/ci/detect-docs-only.sh', 'scripts/ci/lib/doc_classify.py',
                     'scripts/ci/detect-release-only.sh', 'scripts/ci/merge-queue-docs-only.py',
                     '.github/scripts/release-validation-reuse.cjs'):
            destination = self.repo / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((fixtures.ROOT / path).read_bytes())
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture trusted detection policy')
        self.base = self.git('rev-parse', 'HEAD')

    def shell(self, script, **env):
        result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], cwd=self.repo,
                                env=dict(self.env, **env), capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def assert_queue_dispatch(self, *, instrumented_macos, release_candidate='false'):
        head = self.git('rev-parse', 'HEAD')
        Path(self.env['GITHUB_OUTPUT']).write_text('')
        self.shell(detect_script(), EVENT_NAME='merge_group', EVENT_SHA=head,
                   MERGE_GROUP_BASE_SHA=self.base, MERGE_GROUP_HEAD_SHA=head,
                   PR_BASE_SHA='', PR_HEAD_SHA='', PUSH_BEFORE_SHA='', HEAD_REF='',
                   HEAD_REPOSITORY='example/project', REPOSITORY='example/project',
                   RUNNER_TEMP=str(self.root / 'runner'), GITHUB_ENV=str(self.root / 'github-env'))
        outputs = dict(line.split('=', 1) for line in Path(self.env['GITHUB_OUTPUT']).read_text().splitlines())
        self.assertEqual(outputs['base_sha'], self.base)
        self.assertEqual(outputs['head_sha'], head)
        self.assertEqual(outputs['docs_only'], 'false', 'queue candidate bypassed the Linux planner')
        self.assertEqual(outputs['release_candidate'], release_candidate)

        workflow = (fixtures.ROOT / '.github/workflows/ci.yml').read_text()
        condition = workflow.split('\n  full_validation:\n', 1)[1].split('    if: ', 1)[1].splitlines()[0]
        context = {'github.event_name': 'merge_group', 'github.ref': 'refs/heads/queue',
                   'needs.changes.result': 'success'}
        expression = re.sub(r'needs\.changes\.outputs\.(\w+)',
                            lambda m: repr(outputs.get(m[1], '')), condition)
        for key, value in context.items():
            expression = expression.replace(key, repr(value))
        selected = eval(expression.replace('always()', 'True').replace('&&', 'and').replace('||', 'or'),
                        {'__builtins__': {}}, {})
        self.assertEqual(selected, instrumented_macos, 'instrumented macOS queue selection')
        self.assertEqual(outputs['queue_docs_only'], str(not instrumented_macos).lower())

    def test_prose_only_queue_preserves_the_fast_lane(self):
        with (self.repo / 'README.md').open('a') as stream:
            stream.write('\nFixture documentation update.\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture docs queue candidate')
        self.assertEqual(self.shell('bash scripts/ci/detect-docs-only.sh --base "$FIXTURE_BASE"',
                                    FIXTURE_BASE=self.base).strip(), 'true')
        self.assert_queue_dispatch(instrumented_macos=False)

    def test_queue_code_shared_inputs_and_workflow_changes_select_macos(self):
        for path in (
            'crates/agent-session/tests/integration/codex_resume_identity.rs',
            'crates/nils-test-support/src/node.rs',
            'crates/agent-session/docs/specs/fixtures/history-row-classes-v1.json',
            'docs/runbooks/review-specialists-primitive.md',
            '.github/workflows/full-validation.yml',
            'scripts/ci/merge-queue-docs-only.py',
        ):
            with self.subTest(path=path):
                self.git('checkout', '-q', '--detach', self.base)
                changed = self.repo / path
                changed.parent.mkdir(parents=True, exist_ok=True)
                # Changing the selector must still evaluate the base copy.
                changed.write_text("print('true')\n" if changed.name == 'merge-queue-docs-only.py'
                                   else 'fixture change\n')
                self.git('add', '.')
                self.git('commit', '-qm', 'fixture queue candidate')
                self.assert_queue_dispatch(instrumented_macos=True)

    def test_prose_with_a_deleted_fixture_selects_macos(self):
        path = 'crates/agent-session/tests/fixtures/session.md'
        fixture = self.repo / path
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_text('fixture\n')
        self.git('add', path)
        self.git('commit', '-qm', 'fixture in queue base')
        self.base = self.git('rev-parse', 'HEAD')
        self.git('rm', path)
        with (self.repo / 'README.md').open('a') as stream:
            stream.write('\nFixture documentation update.\n')
        self.git('add', 'README.md')
        self.git('commit', '-qm', 'fixture removal and prose')
        self.assert_queue_dispatch(instrumented_macos=True)

    def test_missing_base_selector_requires_macos_even_for_prose(self):
        path = 'scripts/ci/merge-queue-docs-only.py'
        self.git('rm', path)
        self.git('commit', '-qm', 'fixture without the trusted selector')
        self.base = self.git('rev-parse', 'HEAD')
        with (self.repo / 'README.md').open('a') as stream:
            stream.write('\nFixture documentation update.\n')
        self.git('add', 'README.md')
        self.git('commit', '-qm', 'fixture prose after a base without the selector')
        self.assert_queue_dispatch(instrumented_macos=True)

    def test_release_only_queue_candidate_forces_full_validation(self):
        self.shell(self.helpers + '\nwrite_release_commit "$FIXTURE_REPO"\n')
        self.git('commit', '--amend', '-qm', 'chore(release): bump cli versions to 1.2.4 (#17)')
        self.assertEqual(self.shell('bash scripts/ci/detect-release-only.sh --base "$FIXTURE_BASE" --merge-group',
                                    FIXTURE_BASE=self.base).strip(), 'true')
        self.assert_queue_dispatch(instrumented_macos=True, release_candidate='true')


if __name__ == '__main__':
    unittest.main(verbosity=2)
