#!/usr/bin/env python3
"""Local-fast scope regressions using a synthetic resolved Cargo graph."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / 'scripts/ci/nils-cli-local-fast.sh'
FIXTURE = Path(__file__).with_name('fixtures') / 'local-fast-metadata.json'


class ScopeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        directory = Path(self.temp.name)
        metadata = FIXTURE.read_text().replace('__REPO__', str(ROOT)).replace(
            '__EXTERNAL__', str(directory / 'external'))
        (directory / 'metadata.json').write_text(metadata)
        self.arguments = directory / 'arguments.json'
        cargo = directory / 'cargo'
        cargo.write_text('''#!/usr/bin/env python3
import json
from pathlib import Path
import sys
root = Path(__file__).parent
(root / 'arguments.json').write_text(json.dumps(sys.argv[1:]))
print((root / 'metadata.json').read_text())
''')
        cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=f'{directory}{os.pathsep}{os.environ["PATH"]}')

    def plan(self, *paths):
        args = [arg for path in paths for arg in ('--changed-file', path)]
        result = subprocess.run(['bash', str(SCRIPT), '--plan-only', *args],
                                cwd=ROOT, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        plan = {}
        for line in result.stdout.splitlines():
            key, separator, value = line.partition('=')
            if separator:
                plan.setdefault(key, []).append(value)
        return plan

    def assert_scope(self, paths, expected):
        plan = self.plan(*paths)
        self.assertEqual(plan['LOCAL_FAST_MODE'], ['packages'])
        self.assertEqual(plan['LOCAL_FAST_PACKAGE'], sorted(expected))
        return plan

    def test_transitive_reverse_dependents_and_dependency_kinds(self):
        plan = self.assert_scope(['crates/base/src/lib.rs'],
                                 ['base', 'mid', 'leaf', 'dev-consumer', 'build-consumer', 'unrelated'])
        reasons = plan['LOCAL_FAST_PACKAGE_REASON']
        self.assertIn('base: changed path: crates/base/src/lib.rs', reasons)
        self.assertIn('mid: reverse dependency of base', reasons)
        self.assertIn('leaf: reverse dependency of mid', reasons)
        self.assertIn('unrelated: reverse dependency of external', reasons)
        self.assertEqual({r.split(':', 1)[0] for r in reasons},
                         set(plan['LOCAL_FAST_PACKAGE']))

    def test_multiple_changes_union_without_duplicates(self):
        self.assert_scope(['crates/mid/src/lib.rs', 'crates/unrelated/src/lib.rs'],
                          ['mid', 'leaf', 'build-consumer', 'unrelated'])

    def test_leaf_does_not_select_forward_dependencies(self):
        self.assert_scope(['crates/leaf/src/lib.rs'], ['leaf'])

    def test_coupling_also_expands_reverse_dependents(self):
        plan = self.assert_scope(['crates/nils-agent-session/src/lib.rs'],
                                 ['nils-agent-session', 'nils-main-agent', 'coupled-consumer'])
        self.assertIn('nils-main-agent: coupled with nils-agent-session',
                      plan['LOCAL_FAST_PACKAGE_REASON'])

    def test_reverse_dependent_can_trigger_binary_coupling(self):
        metadata_path = Path(self.temp.name) / 'metadata.json'
        metadata = json.loads(metadata_path.read_text())
        session = next(n for n in metadata['resolve']['nodes'] if n['id'] == 'nils-agent-session')
        session['deps'].append({'pkg': 'leaf', 'name': 'leaf', 'dep_kinds': []})
        metadata_path.write_text(json.dumps(metadata))
        self.assert_scope(['crates/leaf/src/lib.rs'],
                          ['leaf', 'nils-agent-session', 'nils-main-agent', 'coupled-consumer'])

    def test_shared_packages_escalate(self):
        metadata_path = Path(self.temp.name) / 'metadata.json'
        metadata = json.loads(metadata_path.read_text())
        shared = ('nils-common', 'nils-term', 'nils-test-support', 'nils-scrub')
        for name in shared:
            metadata['packages'].append({'id': name, 'name': name,
                                        'manifest_path': str(ROOT / f'crates/{name}/Cargo.toml'),
                                        'targets': []})
            metadata['workspace_members'].append(name)
            metadata['resolve']['nodes'].append({'id': name, 'deps': []})
        metadata_path.write_text(json.dumps(metadata))
        for name in shared:
            with self.subTest(package=name):
                plan = self.plan(f'crates/{name}/src/lib.rs')
                self.assertEqual(plan['LOCAL_FAST_MODE'], ['workspace'])
                self.assertIn(f'shared package changed: {name}', plan['LOCAL_FAST_REASON'])

    def test_metadata_resolves_dependencies_without_network_or_lock_changes(self):
        self.plan('crates/leaf/src/lib.rs')
        args = json.loads(self.arguments.read_text())
        self.assertNotIn('--no-deps', args)
        self.assertIn('--offline', args)
        self.assertIn('--locked', args)
        self.assertIn('--all-features', args)

    def test_build_and_graph_inputs_escalate(self):
        for path in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml',
                     '.cargo/config', '.cargo/config.toml', 'crates/mid/.cargo/config.toml',
                     'crates/mid/Cargo.toml',
                     'crates/mid/build.rs', 'crates/mid/build/custom.rs'):
            with self.subTest(path=path):
                # A custom build target's source need not be named build.rs.
                metadata_path = Path(self.temp.name) / 'metadata.json'
                metadata = json.loads(metadata_path.read_text())
                mid = next(p for p in metadata['packages'] if p['id'] == 'mid')
                mid['targets'].append({'kind': ['custom-build'], 'doctest': False,
                                      'src_path': str(ROOT / 'crates/mid/build/custom.rs')})
                metadata_path.write_text(json.dumps(metadata))
                plan = self.plan(path)
                self.assertEqual(plan['LOCAL_FAST_MODE'], ['workspace'])

    def test_docs_only_does_not_read_graph(self):
        plan = self.plan('docs/runbooks/example.md', 'crates/mid/README.md')
        self.assertEqual(plan['LOCAL_FAST_MODE'], ['docs-only'])
        self.assertNotIn('LOCAL_FAST_PACKAGE', plan)
        self.assertFalse(self.arguments.exists())


if __name__ == '__main__':
    unittest.main()
