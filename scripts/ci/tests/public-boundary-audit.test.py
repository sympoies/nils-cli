#!/usr/bin/env python3
"""Public boundary audit regressions: committed credentials and user-home paths."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
AUDIT = ROOT / 'scripts/ci/public-boundary-audit.py'
ALLOWLIST = 'scripts/ci/public-boundary-allowlist.tsv'

# Synthetic secrets are built at run time so this file never contains one.
GITHUB_TOKEN = 'gh' + 'p_' + 'a' * 36
AWS_KEY_ID = 'AK' + 'IA' + 'B' * 16
PEM_HEADER = '-----BEGIN ' + 'RSA PRIVATE KEY-----'
HOME_PREFIX = '/' + 'home'
USERS_PREFIX = '/' + 'Users'


class AuditTests(unittest.TestCase):
    def audit(self, files, allowlist=''):
        with tempfile.TemporaryDirectory(prefix='public-boundary-audit-') as directory:
            root = Path(directory)
            files = dict(files)
            files[ALLOWLIST] = allowlist
            for name, text in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
            subprocess.run(['git', 'add', '-A'], cwd=root, check=True)
            return subprocess.run([sys.executable, str(AUDIT), '--root', str(root)],
                                  text=True, capture_output=True)

    def test_check_chains_run_audit(self):
        for script in ('scripts/ci/nils-cli-local-fast.sh',
                       'scripts/ci/nils-cli-scoped-checks.sh',
                       'scripts/ci/nils-cli-verification.sh'):
            with self.subTest(script=script):
                source = (ROOT / script).read_text()
                for call in ('run python3 scripts/ci/public-boundary-audit.py',
                             'run python3 scripts/ci/tests/public-boundary-audit.test.py'):
                    self.assertTrue(call in source, f'{script} must run: {call}')

    def test_absolute_home_paths_fail_for_both_platform_prefixes(self):
        for prefix in ('/home', '/Users'):
            with self.subTest(prefix=prefix):
                result = self.audit({'docs/note.md': f'open {prefix}/synthetic-owner/notes.txt\n'})
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f'docs/note.md:1: absolute home path {prefix}/synthetic-owner',
                              result.stdout)

    def test_listed_placeholder_users_and_non_absolute_forms_pass(self):
        files = {
            # Built at run time so this file holds no absolute home path itself.
            'a.rs': f'let p = "{HOME_PREFIX}/placeholder/x";\nlet q = "{USERS_PREFIX}/placeholder/y";\n',
            'b.md': 'relative tmp/home/synthetic-owner and ~/notes and /home/$USER and /home/{}\n',
            # A token shape inside a longer identifier is not a token.
            'c.rs': 'fn fresh_sidecar_outweighs_an_old_registry_projection_timestamp() {}\n',
        }
        result = self.audit(files, 'home-user\tplaceholder\tsynthetic placeholder user\n')
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_credential_shapes_fail(self):
        for label, secret in (('github token', GITHUB_TOKEN), ('aws access key id', AWS_KEY_ID),
                              ('private key block', PEM_HEADER)):
            with self.subTest(label=label):
                result = self.audit({'src/config.txt': f'x = {secret}\n'})
                self.assertEqual(result.returncode, 1, result.stdout)
                self.assertIn(f'src/config.txt:1: committed credential ({label})', result.stdout)
                self.assertNotIn(secret, result.stdout + result.stderr)

    def test_credential_exception_applies_to_its_exact_path_only(self):
        files = {'fixtures/scrub.txt': GITHUB_TOKEN + '\n', 'src/other.txt': GITHUB_TOKEN + '\n'}
        result = self.audit(files, 'credential-file\tfixtures/scrub.txt\tsynthetic scrubber input\n')
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn('src/other.txt:1:', result.stdout)
        self.assertNotIn('fixtures/scrub.txt', result.stdout)

    def test_exception_rows_need_a_known_kind_and_a_reason(self):
        for row in ('credential-file\tfixtures/scrub.txt\n', 'unknown-kind\tx\tbecause\n'):
            with self.subTest(row=row):
                result = self.audit({'a.txt': 'plain\n'}, row)
                self.assertEqual(result.returncode, 2, result.stdout)
                self.assertIn(f'{ALLOWLIST}:1:', result.stderr)


if __name__ == '__main__':
    unittest.main()
