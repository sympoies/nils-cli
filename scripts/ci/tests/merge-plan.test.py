#!/usr/bin/env python3
"""Exact-tree selector contracts; fixtures never need a registry or network."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import shutil
import tempfile
import unittest
from unittest import mock

import jsonschema

ROOT = Path(__file__).resolve().parents[3]
LIB = ROOT / 'scripts/ci/lib/merge_plan.py'
LAUNCHER = ROOT / 'scripts/ci/nils-cli-merge-plan.py'
POLICY = ROOT / 'scripts/ci/merge-plan-policy.v1.json'
REGISTRY = ROOT / 'scripts/ci/merge-plan-extra-inputs.v1.json'
SCHEMA = ROOT / 'scripts/ci/merge-plan.schema.v2.json'


def graph(root, edges=()):
    names = ('base', 'mid', 'leaf', 'dev', 'build', 'optional', 'target', 'unrelated')
    packages = [{'id': f'path+{root.as_uri()}/crates/{name}#{name}@0.1.0',
                 'name': name, 'manifest_path': str(root / f'crates/{name}/Cargo.toml'),
                 'targets': []} for name in names]
    ids = {p['name']: p['id'] for p in packages}
    nodes = [{'id': ids[name], 'deps': []} for name in names]
    for consumer, dependency, kind, target in edges:
        node = next(n for n in nodes if n['id'] == ids[consumer])
        node['deps'].append({'name': 'renamed_import', 'pkg': ids[dependency],
                             'dep_kinds': [{'kind': kind, 'target': target}]})
    return {'workspace_root': str(root), 'workspace_members': list(ids.values()),
            'packages': packages, 'resolve': {'nodes': nodes}}


class SelectorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not LIB.exists():
            raise AssertionError('D1 selector contract is missing: no versioned merge plan')
        spec = importlib.util.spec_from_file_location('merge_plan', LIB)
        cls.mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.mod)

    def setUp(self):
        self.root = Path('/fixture/repository')
        self.policy = json.loads(POLICY.read_text())
        self.registry = {'schema_version': 'nils-cli.merge-extra-inputs.v1', 'rules': []}
        self.base = graph(self.root)
        self.candidate = copy.deepcopy(self.base)

    def select(self, changes, base=None, candidate=None):
        return self.mod.select(changes, self.base if base is None else base,
                               self.candidate if candidate is None else candidate,
                               self.policy, self.registry)

    def names(self, result):
        return [p['name'] for p in result['selected_packages']]

    def test_all_dependency_kinds_and_transitive_union(self):
        edges = [('mid', 'base', None, None), ('leaf', 'mid', None, None),
                 ('dev', 'base', 'dev', None), ('build', 'base', 'build', None),
                 ('optional', 'base', None, None), ('target', 'base', None, 'cfg(windows)')]
        base = graph(self.root, edges)
        candidate = graph(self.root, [('unrelated', 'leaf', None, None)])
        result = self.select([{'status': 'M', 'paths': ['crates/base/src/lib.rs']}], base, candidate)
        self.assertEqual(result['scope'], 'FULL')
        self.assertIn('ALL_WORKSPACE_PACKAGES', [r['code'] for r in result['reasons']])
        self.assertEqual(set(self.names(result)), set(('base', 'mid', 'leaf', 'dev', 'build',
                                                     'optional', 'target', 'unrelated')))

    def test_proper_subset_reverse_closure_remains_scoped(self):
        metadata = graph(self.root, [('mid', 'base', None, None), ('leaf', 'mid', None, None)])
        result = self.select([{'status': 'M', 'paths': ['crates/base/src/lib.rs']}], metadata, metadata)
        self.assertEqual(result['scope'], 'SCOPED')
        self.assertEqual(set(self.names(result)), {'base', 'mid', 'leaf'})

    def test_public_schema_contracts_escalate_before_owner_or_docs(self):
        contracts = (
            'crates/agent-workflow-primitives/docs/specs/test-first-evidence-mutation-receipt-v1.schema.json',
            'crates/codex-cli/docs/specs/execution-capsule-receipt-v1.schema.json',
            'crates/codex-cli/docs/specs/execution-capsule-error-v1.schema.json',
            'crates/leaf/docs/specs/response-v1.schema.json',
        )
        for path in contracts:
            with self.subTest(path=path):
                result = self.select([{'status': 'M', 'paths': [path]}])
                self.assertEqual(result['scope'], 'FULL')
                self.assertIn('ALWAYS_ESCALATE', [r['code'] for r in result['reasons']])
        control = self.select([{'status': 'M', 'paths': ['crates/leaf/tests/fixtures/response.schema.json']}])
        self.assertEqual(control['scope'], 'SCOPED')
        self.assertEqual(self.names(control), ['leaf'])

    def test_external_intermediary_is_traversed(self):
        external = {'id': 'registry+example#external@1.0.0', 'name': 'external',
                    'manifest_path': '/registry/external/Cargo.toml', 'targets': []}
        for metadata in (self.base, self.candidate):
            metadata['packages'].append(external)
            source = metadata['workspace_members'][0]
            consumer = metadata['workspace_members'][-1]
            metadata['resolve']['nodes'].append({'id': external['id'],
                                                'deps': [{'pkg': source}]})
            metadata['resolve']['nodes'][-2]['deps'].append({'pkg': external['id']})
        self.assertEqual(set(self.names(self.select([{'status': 'M', 'paths': ['crates/base/src/lib.rs']}]))),
                         {'base', 'unrelated'})

    def test_rename_selects_both_paths_without_duplicates(self):
        changes = self.mod.parse_diff(b'R100\0crates/base/src/old\0crates/leaf/src/new\0')
        result = self.select(changes)
        self.assertEqual(set(self.names(result)), {'base', 'leaf'})
        self.assertEqual(len(result['selected_packages']), 2)

    def test_nul_paths_and_deletion(self):
        changes = self.mod.parse_diff(b'D\0crates/base/src/a\nwith\ttabs.rs\0')
        self.assertEqual(changes[0]['paths'], ['crates/base/src/a\nwith\ttabs.rs'])
        self.assertEqual(self.names(self.select(changes)), ['base'])

    def test_deleted_or_unmappable_package_escalates(self):
        candidate = graph(self.root)
        deleted = candidate['workspace_members'].pop(0)
        candidate['packages'] = [p for p in candidate['packages'] if p['id'] != deleted]
        candidate['resolve']['nodes'] = [n for n in candidate['resolve']['nodes'] if n['id'] != deleted]
        self.assertEqual(self.select([{'status': 'D', 'paths': ['crates/base/src/lib.rs']}],
                                     candidate=candidate)['scope'], 'FULL')
        self.assertEqual(self.select([{'status': 'M', 'paths': ['crates/missing/src/lib.rs']}])['scope'], 'FULL')

    def test_mode_and_symlink_type_changes_select_owner(self):
        for status in ('M', 'T'):
            with self.subTest(status=status):
                self.assertEqual(self.names(self.select([{'status': status,
                                                         'paths': ['crates/leaf/src/lib.rs']}])) , ['leaf'])

    def test_extra_inputs_cross_binary_fixture_and_generated(self):
        self.registry['rules'] = [
            {'id': 'binary', 'paths': ['crates/leaf/**'], 'packages': ['base']},
            {'id': 'fixture', 'paths': ['fixtures/runtime/**'], 'packages': ['mid']},
            {'id': 'generated', 'paths': ['generated/**'], 'packages': ['leaf']},
        ]
        for path, names in [('crates/leaf/src/main.rs', {'base', 'leaf'}),
                            ('fixtures/runtime/data.json', {'mid'}), ('generated/input.inc', {'leaf'})]:
            with self.subTest(path=path):
                result = self.select([{'status': 'M', 'paths': [path]}])
                self.assertEqual(result['scope'], 'SCOPED')
                self.assertEqual(set(self.names(result)), names)
        self.registry['rules'][0]['packages'] = ['missing']
        self.assertEqual(self.select([{'status': 'M', 'paths': ['crates/leaf/src/main.rs']}])['scope'], 'FULL')

    def test_forge_cli_integration_binary_follows_its_scoped_dependencies(self):
        # forge-cli integration tests run the semantic-commit binary as a sibling.
        # A change to a forge-cli dependency selects forge-cli by reverse closure
        # but would not build that sibling, so the scoped lane fails with
        # NILS_TEST_REQUIRE_SIBLING_BINS=1. Dependencies outside always-escalate
        # must select it explicitly.
        rules = json.loads(REGISTRY.read_text())['rules']
        covered = {path for rule in rules if 'nils-semantic-commit' in rule['packages']
                   for path in rule['paths']}
        for path in ('crates/agent-workflow-primitives/**', 'crates/nils-build-info/**'):
            with self.subTest(path=path):
                self.assertIn(path, covered)

    def test_extra_input_reverse_closure(self):
        self.registry['rules'] = [{'id': 'include', 'paths': ['includes/**'], 'packages': ['base']}]
        metadata = graph(self.root, [('leaf', 'base', None, None)])
        self.assertEqual(set(self.names(self.select([{'status': 'M', 'paths': ['includes/file']}],
                                                     base=metadata, candidate=metadata))), {'base', 'leaf'})

    def test_always_escalate_before_docs_or_registry(self):
        paths = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates/base/build.rs',
                 'crates/base/Cargo.toml', 'crates/base/.cargo/config.toml',
                 '.github/workflows/test.yml', 'scripts/release.sh', 'tests/support.rs',
                 'crates/nils-test-support/src/lib.rs', 'crates/agent-session/README.md',
                 'crates/forge-cli/src/auth.rs', '.agents/config.json']
        self.registry['rules'] = [{'id': 'overlap', 'paths': ['**'], 'packages': ['leaf']}]
        for path in paths:
            with self.subTest(path=path):
                result = self.select([{'status': 'M', 'paths': [path]}])
                self.assertEqual(result['scope'], 'FULL')
                self.assertIn('ALWAYS_ESCALATE', [r['code'] for r in result['reasons']])

    def test_custom_build_target(self):
        self.base['packages'][0]['targets'] = [{'kind': ['custom-build'],
                                                'src_path': str(self.root / 'crates/base/custom/script.rs')}]
        self.assertEqual(self.select([{'status': 'M', 'paths': ['crates/base/custom/script.rs']}])['scope'], 'FULL')

    def test_unknown_paths_full_and_empty_none(self):
        self.assertEqual(self.select([{'status': 'M', 'paths': ['unknown/input']}])['scope'], 'FULL')
        self.assertEqual(self.select([])['scope'], 'NONE')

    def test_docs_known_paths_and_deterministic_order(self):
        changes = [{'status': 'M', 'paths': ['docs/specs/example.md']},
                   {'status': 'M', 'paths': ['crates/base/src/lib.rs']}]
        a = self.select(changes)
        b = self.select(list(reversed(changes)))
        self.assertEqual(a, b)
        self.assertEqual(self.select(changes[:1])['selected_packages'], [])

    def test_incomplete_external_node_is_metadata_failure(self):
        external = {'id': 'registry+example#external@1.0.0', 'name': 'external'}
        self.base['packages'].append(external)
        with self.assertRaises(self.mod.PlanError):
            self.select([], base=self.base)

    def test_policy_and_registry_versions_fail_closed(self):
        for key in ('always_escalate', 'documentation'):
            original = self.policy.pop(key)
            with self.assertRaises(self.mod.PlanError):
                self.select([])
            self.policy[key] = original
        self.registry['schema_version'] = 'unknown'
        with self.assertRaises(self.mod.PlanError):
            self.select([])

    def test_malformed_graph_and_diff_fail_closed(self):
        for metadata in ({}, dict(self.base, resolve=None),
                         dict(self.base, workspace_members=['not-in-packages'])):
            with self.subTest(metadata=metadata):
                with self.assertRaises(self.mod.PlanError):
                    self.select([], base=metadata)
        for raw in (b'R100\0old\0', b'M\0../escape\0', b'U\0path\0', b'M\0unterminated'):
            with self.subTest(raw=raw):
                with self.assertRaises(self.mod.PlanError):
                    self.mod.parse_diff(raw)


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(LAUNCHER.exists(), 'versioned exact-commit planner command is missing')
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / 'repo'
        self.repo.mkdir()
        self.env = dict(os.environ, GIT_AUTHOR_NAME='Fixture', GIT_AUTHOR_EMAIL='fixture@example.invalid',
                        GIT_COMMITTER_NAME='Fixture', GIT_COMMITTER_EMAIL='fixture@example.invalid',
                        GIT_AUTHOR_DATE='2000-01-01T00:00:00Z', GIT_COMMITTER_DATE='2000-01-01T00:00:00Z')
        # This baseline fake Cargo models offline planning even under CI's opt-in.
        self.env['NILS_CLI_MERGE_PLAN_PREPARE_METADATA'] = '0'
        self.git('init', '-q')
        for source in (LIB, LAUNCHER, POLICY, REGISTRY, SCHEMA):
            target = self.repo / source.relative_to(ROOT)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())
        (self.repo / 'crates/leaf/src').mkdir(parents=True)
        (self.repo / 'crates/leaf/src/lib.rs').write_text('fixture')
        self.base = self.commit()
        (self.repo / 'crates/leaf/src/lib.rs').write_text('changed')
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        self.tools = Path(self.temp.name) / 'tools'
        self.tools.mkdir()
        cargo = self.tools / 'cargo'
        cargo.write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
sys.path.insert(0, os.environ['TEST_SUITE_DIR'])
import importlib.util
spec = importlib.util.spec_from_file_location('suite', Path(os.environ['TEST_SUITE_DIR'])/'merge-plan.test.py')
suite = importlib.util.module_from_spec(spec)
spec.loader.exec_module(suite)
snapshot = Path(sys.argv[sys.argv.index('--manifest-path') + 1]).parent
assert Path.cwd().name == 'base'
assert sys.argv[1:] == ['metadata', '--manifest-path', str(snapshot/'Cargo.toml'), '--offline', '--locked', '--all-features', '--format-version', '1']
assert os.environ['RUSTC_WRAPPER'] == os.environ['RUSTC_WORKSPACE_WRAPPER'] == ''
if os.environ.get('FAIL_METADATA') or (os.environ.get('FAIL_CANDIDATE') and snapshot.name == 'candidate'):
    print('/private/fixture sensitive metadata diagnostic', file=sys.stderr)
    sys.exit(101)
if os.environ.get('BAD_METADATA'):
    print('{}')
else:
    print(json.dumps(suite.graph(snapshot)))
''')
        cargo.chmod(0o755)
        self.env.update(PATH=f'{self.tools}{os.pathsep}{os.environ["PATH"]}',
                        TEST_SUITE_DIR=str(Path(__file__).parent))

    def git(self, *args, input=None):
        return subprocess.check_output(['git', *args], cwd=self.repo, env=self.env, input=input).decode().strip()

    def commit(self, *parents):
        self.git('add', '.')
        tree = self.git('write-tree')
        return self.git('commit-tree', tree, *[arg for parent in parents for arg in ('-p', parent)],
                        input=b'fixture\n')

    def run_plan(self, *extra, **env):
        return subprocess.run(['python3', str(LAUNCHER), '--repo', str(self.repo),
                               '--base', self.base, '--head', self.head, '--candidate', self.candidate, *extra],
                              env=dict(self.env, **env), capture_output=True, text=True)

    def selector_module(self):
        spec = importlib.util.spec_from_file_location('snapshot_selector', LIB)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_snapshot_rejects_case_and_unicode_aliases_before_writes(self):
        module = self.selector_module()
        for first, second in (('A', 'a'), ('dir/A', 'dir/a'), ('é', 'e\u0301'),
                              ('Dir/a', 'dir/b'), ('A', 'a/file')):
            with self.subTest(names=(first, second)):
                destination = Path(self.temp.name) / ('snapshot-' + str(len(first)))
                destination.mkdir(exist_ok=True)
                entries = (f'120000 blob one\t{first}\0' + f'100644 blob two\t{second}\0').encode()
                def read_git(repo, *args):
                    return entries if args[0] == 'ls-tree' else b'safe.txt'
                with mock.patch.object(module, 'git', side_effect=read_git):
                    with self.assertRaises(module.PlanError):
                        module.snapshot(self.repo, self.candidate, destination)
                self.assertEqual(list(destination.iterdir()), [])

    def test_snapshot_parent_symlink_cannot_overwrite_external_fixture(self):
        module = self.selector_module()
        outside = Path(self.temp.name) / 'outside'
        victim = outside / 'leaf/src/lib.rs'
        victim.parent.mkdir(parents=True)
        victim.write_text('preserved')
        destination = Path(self.temp.name) / 'snapshot'
        destination.mkdir()
        (destination / 'crates').symlink_to(outside, target_is_directory=True)
        with self.assertRaises(module.PlanError):
            module.snapshot(self.repo, self.candidate, destination)
        self.assertEqual(victim.read_text(), 'preserved')

    def test_snapshot_external_symlink_is_hold_without_touching_target(self):
        victim = Path(self.temp.name) / 'outside-target'
        victim.write_text('preserved')
        (self.repo / 'crates/leaf/src/link').symlink_to(victim)
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        result = self.run_plan()
        plan = json.loads(result.stdout)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(plan['error']['code'], 'SNAPSHOT_FAILED')
        self.assertEqual(plan['action'], 'HOLD')
        self.assertEqual(victim.read_text(), 'preserved')

    def test_snapshot_safe_relative_links_and_executable_modes(self):
        module = self.selector_module()
        source = self.repo / 'crates/leaf/src/lib.rs'
        source.chmod(0o755)
        (source.parent / 'link').symlink_to('lib.rs')
        head = self.commit(self.base)
        destination = Path(self.temp.name) / 'snapshot'
        destination.mkdir()
        module.snapshot(self.repo, head, destination)
        copied = destination / 'crates/leaf/src/lib.rs'
        self.assertTrue(copied.stat().st_mode & 0o111)
        self.assertTrue((copied.parent / 'link').is_symlink())
        self.assertEqual((copied.parent / 'link').read_text(), copied.read_text())

    def real_minimal_workspace(self):
        cargo = shutil.which('cargo', path=os.environ['PATH'])
        self.assertIsNotNone(cargo)
        (self.repo / 'Cargo.toml').write_text(
            '[workspace]\nresolver="2"\nmembers=["crates/leaf", "crates/mid"]\n')
        for name in ('leaf', 'mid'):
            root = self.repo / f'crates/{name}'
            (root / 'src').mkdir(parents=True, exist_ok=True)
            (root / 'src/lib.rs').write_text('')
            (root / 'Cargo.toml').write_text(
                f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2021"\n')
        subprocess.run([cargo, 'generate-lockfile', '--offline'], cwd=self.repo, env=os.environ,
                       check=True, capture_output=True)
        self.env['PATH'] = os.environ['PATH']
        # Do not let ambient compiler overrides mask the compiler sentinel.
        for key in ('RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'CARGO_BUILD_RUSTC',
                    'CARGO_BUILD_RUSTC_WRAPPER', 'CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER'):
            self.env.pop(key, None)

    def sentinel_compiler(self):
        marker = Path(self.temp.name) / 'compiler-ran'
        compiler = self.repo / 'tools/compiler'
        compiler.parent.mkdir(exist_ok=True)
        compiler.write_text('#!/usr/bin/env python3\nfrom pathlib import Path\n' +
                            f'Path({str(marker)!r}).write_text("executed")\nraise SystemExit(101)\n')
        compiler.chmod(0o755)
        return compiler, marker

    def test_candidate_compiler_config_is_held_without_execution(self):
        self.real_minimal_workspace()
        self.base = self.commit()
        compiler, marker = self.sentinel_compiler()
        (self.repo / '.cargo').mkdir()
        (self.repo / '.cargo/config.toml').write_text('[build]\nrustc = ' + json.dumps(str(compiler)) + '\n')
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        result = self.run_plan()
        self.assertFalse(marker.exists(), 'candidate-selected compiler executed during planning')
        self.assertEqual(result.returncode, 1)
        plan = json.loads(result.stdout)
        self.assertEqual((plan['status'], plan['action']), ('PLAN_ERROR', 'HOLD'))
        self.assertEqual(plan['error']['code'], 'METADATA_FAILED')

    def test_unchanged_base_compiler_config_cannot_select_candidate_program(self):
        self.real_minimal_workspace()
        compiler, marker = self.sentinel_compiler()
        rustc = shutil.which('rustc', path=os.environ['PATH'])
        self.assertIsNotNone(rustc)
        compiler.write_text('#!/usr/bin/env python3\nimport os, sys\n' +
                            f'os.execv({rustc!r}, [{rustc!r}, *sys.argv[1:]])\n')
        (self.repo / '.cargo').mkdir()
        (self.repo / '.cargo/config.toml').write_text('[build]\nrustc = "./tools/compiler"\n')
        self.base = self.commit()
        self.sentinel_compiler()
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        result = self.run_plan()
        self.assertFalse(marker.exists(), 'candidate program ran through unchanged base configuration')
        self.assertEqual(result.returncode, 1, result.stderr)
        plan = json.loads(result.stdout)
        self.assertEqual((plan['status'], plan['action']), ('PLAN_ERROR', 'HOLD'))
        self.assertEqual(plan['error']['stage'], 'candidate_configuration')

    def test_pinned_inputs_trusted_selector_and_deterministic_output(self):
        first = self.run_plan()
        self.assertEqual(first.returncode, 0, first.stderr)
        plan = json.loads(first.stdout)
        jsonschema.Draft202012Validator.check_schema(json.loads(SCHEMA.read_text()))
        jsonschema.validate(plan, json.loads(SCHEMA.read_text()))
        self.assertEqual(plan['status'], 'READY')
        self.assertEqual(plan['scope'], 'SCOPED')
        self.assertEqual([p['name'] for p in plan['selected_packages']], ['leaf'])
        self.assertEqual(plan['inputs']['base'], self.base)
        self.assertEqual(plan['inputs']['head'], self.head)
        self.assertEqual(plan['inputs']['candidate'], self.candidate)
        self.assertEqual(plan['inputs']['tree'], self.git('rev-parse', self.candidate+'^{tree}'))
        (self.repo / LIB.relative_to(ROOT)).write_text('raise RuntimeError("untrusted")')
        (self.repo / POLICY.relative_to(ROOT)).write_text('{}')
        self.assertEqual(first.stdout, self.run_plan().stdout)
        self.assertNotIn(self.temp.name, first.stdout)
        self.assertTrue(all(len(digest) == 64 for digest in plan['digests'].values()))

    def test_metadata_failure_or_invalid_graph_is_hold_never_empty_pass(self):
        for env in ({'FAIL_METADATA': '1'}, {'BAD_METADATA': '1'}, {'FAIL_CANDIDATE': '1'}):
            with self.subTest(env=env):
                result = self.run_plan(**env)
                self.assertEqual(result.returncode, 1, result.stderr)
                plan = json.loads(result.stdout)
                self.assertEqual((plan['status'], plan['action'], plan['scope']), ('PLAN_ERROR', 'HOLD', None))
                jsonschema.validate(plan, json.loads(SCHEMA.read_text()))
                self.assertEqual(plan['error']['code'], 'METADATA_FAILED')
                self.assertNotIn('/private/fixture', result.stdout + result.stderr)
                self.assertNotIn('Traceback', result.stderr)

    def test_real_offline_all_feature_target_dev_build_graph(self):
        # Real Cargo graph: all packages are path dependencies, no network/cache.
        cargo = shutil.which('cargo', path=os.environ['PATH'])
        self.assertIsNotNone(cargo)
        names = ('base', 'mid', 'leaf', 'dev', 'build', 'optional', 'target', 'unrelated')
        manifest = '[workspace]\nresolver = "2"\nmembers = [' + ','.join(
            f'"crates/{name}"' for name in names) + ']\n'
        (self.repo / 'Cargo.toml').write_text(manifest)
        edges = {
            'mid': '[dependencies]\nrenamed = { package="base", path="../base" }\n',
            'leaf': '[dependencies]\nmid = { path="../mid" }\n',
            'dev': '[dev-dependencies]\nbase = { path="../base" }\n',
            'build': '[build-dependencies]\nbase = { path="../base" }\n',
            'optional': '[dependencies]\nbase = { path="../base", optional=true }\n',
            'target': "[target.'cfg(target_os = \"windows\")'.dependencies]\nbase = { path=\"../base\" }\n",
        }
        for name in names:
            root = self.repo / f'crates/{name}'
            (root / 'src').mkdir(parents=True, exist_ok=True)
            (root / 'src/lib.rs').write_text('')
            (root / 'Cargo.toml').write_text(
                f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2021"\n' + edges.get(name, ''))
        subprocess.run([cargo, 'generate-lockfile', '--offline'], cwd=self.repo, env=os.environ,
                       check=True, capture_output=True)
        self.base = self.commit()
        (self.repo / 'crates/base/src/lib.rs').write_text('pub fn changed() {}')
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        self.env['PATH'] = os.environ['PATH']
        first = self.run_plan()
        self.assertEqual(first.returncode, 0, first.stderr)
        plan = json.loads(first.stdout)
        self.assertEqual(plan['scope'], 'SCOPED')
        self.assertEqual({p['name'] for p in plan['selected_packages']}, set(names) - {'unrelated'})
        self.assertEqual(first.stdout, self.run_plan().stdout)
        jsonschema.validate(plan, json.loads(SCHEMA.read_text()))

    def test_non_oid_input_and_wrong_tree_hold(self):
        result = self.run_plan('--tree', '0' * 40)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(json.loads(result.stdout)['error']['code'], 'CANDIDATE_MISMATCH')
        self.base = 'main'
        result = self.run_plan()
        self.assertEqual(result.returncode, 1)
        plan = json.loads(result.stdout)
        self.assertEqual(plan['error']['code'], 'INPUT_INVALID')
        jsonschema.validate(plan, json.loads(SCHEMA.read_text()))

    def test_missing_trusted_selector_has_no_candidate_fallback(self):
        (self.repo / LIB.relative_to(ROOT)).unlink()
        self.base = self.commit()
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        result = self.run_plan()
        self.assertEqual(result.returncode, 1)
        plan = json.loads(result.stdout)
        self.assertEqual(plan['error']['code'], 'SELECTOR_FAILED')
        jsonschema.validate(plan, json.loads(SCHEMA.read_text()))

    def test_type_and_mode_changes_from_exact_base(self):
        link = self.repo / 'crates/leaf/src/link'
        link.symlink_to('lib.rs')
        self.base = self.commit()
        link.unlink()
        link.write_text('regular file')
        (self.repo / 'crates/leaf/src/lib.rs').chmod(0o755)
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        result = self.run_plan()
        self.assertEqual(result.returncode, 0, result.stderr)
        plan = json.loads(result.stdout)
        self.assertEqual(plan['scope'], 'SCOPED')
        self.assertEqual({c['status'] for c in plan['changes']}, {'M', 'T'})

    def test_wrong_candidate_parents_hold(self):
        self.candidate = self.head
        result = self.run_plan()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(json.loads(result.stdout)['error']['code'], 'CANDIDATE_MISMATCH')

    def test_candidate_policy_and_selector_edits_use_base_and_escalate(self):
        (self.repo / LIB.relative_to(ROOT)).write_text('raise RuntimeError("candidate selector")')
        (self.repo / POLICY.relative_to(ROOT)).write_text('{}')
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        result = self.run_plan()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)['scope'], 'FULL')

    def test_cli_diff_retains_deletion_rename_type_and_mode(self):
        old = self.repo / 'crates/leaf/src/lib.rs'
        old.write_text('fixture')
        old.rename(old.with_name('renamed.rs'))
        old.with_name('renamed.rs').chmod(0o755)
        (self.repo / 'crates/leaf/src/link').symlink_to('renamed.rs')
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        first = json.loads(self.run_plan().stdout)
        self.assertEqual(first['scope'], 'SCOPED')
        self.assertTrue(any(c['status'].startswith('R') for c in first['changes']))
        old.with_name('renamed.rs').unlink()
        (self.repo / 'crates/leaf/src/link').unlink()
        (self.repo / 'crates/leaf/src/link').write_text('regular')
        self.head = self.commit(self.base)
        self.candidate = self.commit(self.base, self.head)
        second = json.loads(self.run_plan().stdout)
        self.assertIn('D', [c['status'] for c in second['changes']])


if __name__ == '__main__':
    unittest.main()
