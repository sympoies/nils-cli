#!/usr/bin/env python3
"""Repository devlog policy, through the contributor docs gate and real CLI."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
VERIFY = '.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh'


class DevlogPolicy(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='devlog-policy-')
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name)
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(('GIT_', 'FORGE_IDENTITY_', 'GITHUB_'))}
        self.env.update(DEVLOG_LAYOUT='fragments', NILS_CLI_DEVLOG_BASE='main')
        self.git('init', '-q', '-b', 'main')
        self.git('config', 'user.name', 'Test Author')
        self.git('config', 'user.email', 'author@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        self.write('docs/devlog/README.md', '# Development log\n\n## Months\n\n- [2020-01](2020-01.md)\n')
        self.write('docs/devlog/2020-01.md', '# Development log - 2020-01\n')
        for path in (VERIFY, 'scripts/ci/test-env.sh', 'scripts/ci/lib/node-scope.sh',
                     'scripts/ci/devlog-check.py', 'scripts/ci/nils-cli-local-fast.sh',
                     'scripts/ci/lib/doc_classify.py', 'scripts/ci/lib/sibling_bins.py',
                     'scripts/dev-env.sh',
                     '.agents/scripts/fold-devlog.sh'):
            if (ROOT / path).exists():
                self.write(path, (ROOT / path).read_text())
        for name in ('docs-placement-audit.sh', 'docs-hygiene-audit.sh',
                     'markdownlint-audit.sh', 'cli-output-contract-lint.sh',
                     'forge-cli-fixture-lint.sh', 'docs-prose-audit.sh',
                     'tests/docs-prose-audit.test.sh'):
            self.write('scripts/ci/' + name, '#!/usr/bin/env bash\nexit 0\n')
        self.commit('Baseline')
        self.git('switch', '-q', '-c', 'chore/change')

    def write(self, path, content):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)

    def run_cmd(self, *args, check=True):
        return subprocess.run(args, cwd=self.repo, env=self.env, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=check)

    def git(self, *args):
        return self.run_cmd('git', *args).stdout.strip()

    def commit(self, subject):
        self.git('add', '.')
        self.git('commit', '-qm', subject)

    def new(self, slug, date='2020-01-02'):
        self.run_cmd('devlog', 'new', '--date', date, '--slug', slug, '--title', slug,
                     '--result', 'An outcome.', '--why', 'A reason.', '--evidence', 'A check.')

    def gate(self):
        return self.run_cmd('bash', VERIFY, '--docs-only', check=False)

    def test_docs_gate_rejects_month_edit(self):
        self.write('docs/devlog/2020-01.md', '# Development log - 2020-01\n\nChanged history.\n')
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('month-file-changed', result.stdout)


    def test_local_fast_preserves_explicit_base_for_docs_checks(self):
        self.new('local-fast')
        self.env.pop('NILS_CLI_DEVLOG_BASE')
        result = self.run_cmd('bash', 'scripts/ci/nils-cli-local-fast.sh', '--base', 'main',
                              '--changed-file', 'docs/devlog/pending/2020-01-02-local-fast.md',
                              check=False)
        self.assertEqual(result.returncode, 0, result.stdout)
        path = self.repo / 'docs/devlog/2020-01.md'
        path.write_text(path.read_text() + '\nChanged history.\n')
        result = self.run_cmd('bash', 'scripts/ci/nils-cli-local-fast.sh', '--base', 'main',
                              '--changed-file', 'docs/devlog/2020-01.md', check=False)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('month-file-changed', result.stdout)

    def test_fragments_merge_without_shared_files(self):
        self.new('first')
        self.commit('First fragment')
        first = self.git('rev-parse', 'HEAD')
        self.git('switch', '-q', '-c', 'chore/other', 'main')
        self.new('second')
        self.commit('Second fragment')
        self.git('merge', '--no-edit', first)
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(len(list((self.repo / 'docs/devlog/pending').glob('*.md'))), 2)
        self.assertEqual(self.git('diff', 'main', '--', 'docs/devlog/2020-01.md',
                                  'docs/devlog/README.md'), '')

    def test_merged_fragment_is_immutable(self):
        self.git('switch', '-q', 'main')
        self.new('merged')
        self.commit('Merged fragment')
        self.git('switch', '-q', 'chore/change')
        self.git('merge', '--ff-only', 'main')
        path = self.repo / 'docs/devlog/pending/2020-01-02-merged.md'
        path.write_text(path.read_text().replace('An outcome.', 'Changed outcome.'))
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('fragment-modified', result.stdout)

    def test_maintenance_fold_is_exact_and_idempotent(self):
        self.git('switch', '-q', 'main')
        self.new('accepted')
        self.new('retained', date='2099-01-01')
        self.commit('Accepted fragments')
        self.git('switch', '-q', '-c', 'chore/devlog-fold-2026-10-20')
        self.run_cmd('devlog', 'fold')
        first = (self.repo / 'docs/devlog/2020-01.md').read_bytes()
        self.run_cmd('devlog', 'fold')
        self.assertEqual((self.repo / 'docs/devlog/2020-01.md').read_bytes(), first)
        self.commit('chore(devlog): fold accepted fragments')
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(first.count(b'## 2020-01-02 - accepted'), 1)
        path = self.repo / 'docs/devlog/2020-01.md'
        path.write_bytes(first + b'\nUnrelated history edit.\n')
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('differ from deterministic fold', result.stdout)

    def test_fold_branch_without_canonical_commit_cannot_fold(self):
        self.git('switch', '-q', 'main')
        self.new('accepted')
        self.commit('Accepted fragment')
        self.git('switch', '-q', '-c', 'chore/devlog-fold-2026-10-20')
        self.run_cmd('devlog', 'fold')
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('month-file-changed', result.stdout)

    def test_fold_cannot_hide_staged_history_edit(self):
        self.git('switch', '-q', '-c', 'chore/devlog-fold-2026-10-20')
        self.git('commit', '--allow-empty', '-qm', 'chore(devlog): fold accepted fragments')
        path = self.repo / 'docs/devlog/2020-01.md'
        original = path.read_bytes()
        path.write_bytes(original + b'\nHidden staged edit.\n')
        self.git('add', 'docs/devlog/2020-01.md')
        path.write_bytes(original)
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('exact fold', result.stdout)

    def test_maintainer_preparation_script_folds_without_committing(self):
        self.git('switch', '-q', 'main')
        self.new('accepted')
        self.new('future', date='2099-01-01')
        self.commit('Accepted fragments')
        self.git('switch', '-q', '-c', 'chore/devlog-fold-2026-10-20')
        before = self.git('rev-parse', 'HEAD')
        self.run_cmd('bash', '.agents/scripts/fold-devlog.sh')
        self.assertEqual(self.git('rev-parse', 'HEAD'), before)
        self.assertFalse((self.repo / 'docs/devlog/pending/2020-01-02-accepted.md').exists())
        self.assertTrue((self.repo / 'docs/devlog/pending/2099-01-01-future.md').exists())
        self.assertIn('## 2020-01-02 - accepted',
                      (self.repo / 'docs/devlog/2020-01.md').read_text())
        self.commit('chore(devlog): fold accepted fragments')
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.run_cmd('bash', '.agents/scripts/fold-devlog.sh')
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_fold_passes_pr_and_merge_queue_snapshots(self):
        self.git('switch', '-q', 'main')
        self.new('accepted')
        self.commit('Accepted fragment')
        self.git('switch', '-q', '-c', 'chore/devlog-fold-2026-10-20')
        self.run_cmd('devlog', 'fold')
        self.commit('chore(devlog): fold accepted fragments')
        fold = self.git('rev-parse', 'HEAD')
        self.git('switch', '-q', '-c', 'ci/pr', 'main')
        self.git('merge', '--no-ff', '-m', 'PR merge snapshot', fold)
        self.env.update(GITHUB_EVENT_NAME='pull_request',
                        GITHUB_HEAD_REF='chore/devlog-fold-2026-10-20')
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.env['GITHUB_EVENT_NAME'] = 'merge_group'
        self.env.pop('GITHUB_HEAD_REF')
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.git('switch', '-q', '-c', 'ci/queue', 'main')
        self.git('merge', '--squash', fold)
        self.commit('chore(devlog): fold accepted fragments (#123)')
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_scheduled_default_branch_check_needs_no_pr_base(self):
        self.git('switch', '-q', '--detach', 'main')
        self.env.update(GITHUB_EVENT_NAME='schedule', NILS_CLI_DEVLOG_BASE='missing-base')
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_feature_fold_and_unavailable_base_fail(self):
        self.git('switch', '-q', 'main')
        self.new('accepted')
        self.commit('Accepted fragment')
        self.git('switch', '-q', 'chore/change')
        self.git('merge', '--ff-only', 'main')
        self.run_cmd('devlog', 'fold')
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('month-file-changed', result.stdout)
        self.env['NILS_CLI_DEVLOG_BASE'] = 'missing-base'
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('missing-base', result.stdout)


if __name__ == '__main__':
    unittest.main()
