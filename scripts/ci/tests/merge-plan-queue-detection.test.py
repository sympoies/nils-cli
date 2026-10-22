#!/usr/bin/env python3
"""Execute queue change detection with real docs and release classifiers."""
import importlib.util
from pathlib import Path
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
                     'scripts/ci/detect-release-only.sh', '.github/scripts/release-ci-gate.cjs'):
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

    def assert_forced_full(self, release_candidate='false'):
        head = self.git('rev-parse', 'HEAD')
        self.shell(detect_script(), EVENT_NAME='merge_group', EVENT_SHA=head,
                   MERGE_GROUP_BASE_SHA=self.base, MERGE_GROUP_HEAD_SHA=head,
                   PR_BASE_SHA='', PR_HEAD_SHA='', PUSH_BEFORE_SHA='', HEAD_REF='',
                   HEAD_REPOSITORY='example/project', REPOSITORY='example/project',
                   RUNNER_TEMP=str(self.root / 'runner'), GITHUB_ENV=str(self.root / 'github-env'))
        outputs = dict(line.split('=', 1) for line in Path(self.env['GITHUB_OUTPUT']).read_text().splitlines())
        self.assertEqual(outputs['base_sha'], self.base)
        self.assertEqual(outputs['head_sha'], head)
        self.assertEqual(outputs['docs_only'], 'false', 'docs-only queue candidate bypassed full validation')
        self.assertEqual(outputs['release_candidate'], release_candidate)

    def test_docs_only_queue_candidate_forces_full_validation(self):
        with (self.repo / 'README.md').open('a') as stream:
            stream.write('\nFixture documentation update.\n')
        self.git('add', '.')
        self.git('commit', '-qm', 'fixture docs queue candidate')
        self.assertEqual(self.shell('bash scripts/ci/detect-docs-only.sh --base "$FIXTURE_BASE"',
                                    FIXTURE_BASE=self.base).strip(), 'true')
        self.assert_forced_full()

    def test_release_only_queue_candidate_forces_full_validation(self):
        self.shell(self.helpers + '\nwrite_release_commit "$FIXTURE_REPO"\n')
        self.git('commit', '--amend', '-qm', 'chore(release): bump cli versions to 1.2.4 (#17)')
        self.assertEqual(self.shell('bash scripts/ci/detect-release-only.sh --base "$FIXTURE_BASE" --merge-group',
                                    FIXTURE_BASE=self.base).strip(), 'true')
        self.assert_forced_full(release_candidate='true')


if __name__ == '__main__':
    unittest.main(verbosity=2)
