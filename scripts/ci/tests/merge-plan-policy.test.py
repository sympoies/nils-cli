#!/usr/bin/env python3
"""Shipped policy and registry contracts for the PR and merge-queue affected scope.

The graph below uses real package and binary names with synthetic edges, so the
checked-in policy and registry are exercised exactly as the trusted base loads them.
"""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
LIB = ROOT / 'scripts/ci/lib/merge_plan.py'
POLICY = ROOT / 'scripts/ci/merge-plan-policy.v1.json'
REGISTRY = ROOT / 'scripts/ci/merge-plan-extra-inputs.v1.json'
REPO = Path('/fixture/repository')

# package -> (binaries, workspace dependencies)
PACKAGES = {
    'nils-common': ((), ()),
    'nils-build-info': ((), ()),
    'nils-agent-workflow-primitives': ((), ('nils-common',)),
    'nils-forge-cli': (('forge-cli',), ('nils-common', 'nils-agent-workflow-primitives')),
    'nils-semantic-commit': (('semantic-commit',), ('nils-common', 'nils-build-info')),
    'nils-agent-hook': (('agent-hook',), ('nils-common',)),
    'nils-agent-session': (('agent-session',), ('nils-common', 'nils-agent-hook')),
    'nils-git-cli': (('git-cli',), ('nils-common',)),
    'nils-codex-cli': (('codex-cli',), ('nils-common',)),
    'nils-memo': (('memo',), ('nils-common',)),
}


def crate_dir(name):
    return 'crates/' + name.removeprefix('nils-')


def graph(packages=None):
    packages = PACKAGES if packages is None else packages
    ids = {name: f'path+{REPO.as_uri()}/{crate_dir(name)}#{name}@0.1.0' for name in packages}
    raw = [{'id': ids[name], 'name': name,
            'manifest_path': str(REPO / crate_dir(name) / 'Cargo.toml'),
            'targets': [{'kind': ['bin'], 'name': binary,
                         'src_path': str(REPO / crate_dir(name) / 'src/main.rs')}
                        for binary in bins]}
           for name, (bins, _) in packages.items()]
    nodes = [{'id': ids[name], 'deps': [{'pkg': ids[dep]} for dep in deps if dep in ids]}
             for name, (_, deps) in packages.items()]
    return {'workspace_root': str(REPO), 'workspace_members': list(ids.values()),
            'packages': raw, 'resolve': {'nodes': nodes}}


class ShippedPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location('merge_plan', LIB)
        cls.mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.mod)

    def setUp(self):
        self.policy = json.loads(POLICY.read_text())
        self.registry = json.loads(REGISTRY.read_text())
        self.base = graph()
        self.candidate = copy.deepcopy(self.base)

    def select(self, *paths, candidate=None):
        changes = [{'status': 'M', 'paths': [path]} for path in paths]
        return self.mod.select(changes, self.base, self.candidate if candidate is None else candidate,
                               self.policy, self.registry)

    def names(self, result):
        return sorted(p['name'] for p in result['selected_packages'])

    def codes(self, result):
        return {r['code'] for r in result['reasons']}

    def test_forge_cli_with_generated_completions_and_baseline_is_exactly_scoped(self):
        result = self.select('crates/forge-cli/src/auth.rs', 'completions/zsh/_forge-cli',
                             'completions/bash/forge-cli', 'scripts/ci/file-size-baseline.tsv')
        self.assertEqual(result['scope'], 'SCOPED')
        self.assertEqual(self.names(result), ['nils-forge-cli'])
        self.assertTrue({'AUDIT_ONLY', 'COMPLETION_OWNER'} <= self.codes(result))

    def test_agent_session_selects_hook_session_binaries_only(self):
        result = self.select('crates/agent-session/src/lib.rs')
        self.assertEqual(result['scope'], 'SCOPED')
        self.assertEqual(self.names(result), ['nils-agent-hook', 'nils-agent-session'])

    def test_semantic_commit_does_not_pull_in_the_common_reverse_closure(self):
        result = self.select('crates/semantic-commit/src/commit.rs')
        self.assertEqual(result['scope'], 'SCOPED')
        self.assertEqual(self.names(result), ['nils-forge-cli', 'nils-semantic-commit'])

    def test_completion_owner_follows_the_binary_not_the_package_name(self):
        result = self.select('completions/zsh/_codex-cli')
        self.assertEqual((result['scope'], self.names(result)), ('SCOPED', ['nils-codex-cli']))

    def test_audit_only_baselines_select_no_package_and_do_not_escalate(self):
        result = self.select('scripts/ci/file-size-baseline.tsv', 'scripts/ci/docs-prose-baseline.tsv',
                             'scripts/ci/test-stale-audit-baseline.tsv')
        self.assertEqual((result['scope'], result['selected_packages']), ('SCOPED', []))
        self.assertEqual(self.codes(result), {'AUDIT_ONLY'})

    def test_unmapped_completion_paths_and_other_scripts_stay_full(self):
        for path in ('completions/zsh/_unknown-tool', 'completions/bash/unknown-tool',
                     'completions/zsh/aliases.zsh', 'completions/bash/aliases.bash',
                     'completions/zsh/_completion-adapter-common.zsh',
                     'completions/zsh/_forge-cli/extra', 'completions/fish/forge-cli',
                     'scripts/ci/nils-cli-scoped-checks.sh', 'scripts/ci/file-size-baseline.tsv.bak',
                     'scripts/ci/lib/merge_plan.py', 'scripts/release.sh'):
            with self.subTest(path=path):
                result = self.select(path)
                self.assertEqual(result['scope'], 'FULL')
                self.assertTrue(self.codes(result) & {'ALWAYS_ESCALATE', 'UNKNOWN_COMPLETION_OWNER'})

    def test_completion_with_deleted_or_ambiguous_owner_is_full(self):
        gone = copy.deepcopy(PACKAGES)
        gone['nils-forge-cli'] = ((), gone['nils-forge-cli'][1])
        twice = copy.deepcopy(PACKAGES)
        twice['nils-memo'] = (('forge-cli',), twice['nils-memo'][1])
        for label, packages in (('deleted', gone), ('ambiguous', twice)):
            with self.subTest(label=label):
                result = self.select('completions/zsh/_forge-cli', candidate=graph(packages))
                self.assertEqual(result['scope'], 'FULL')
                self.assertIn('UNKNOWN_COMPLETION_OWNER', self.codes(result))

    def test_lockfile_manifest_and_shared_foundation_stay_full(self):
        for path in ('Cargo.lock', 'Cargo.toml', 'crates/forge-cli/Cargo.toml',
                     'crates/nils-common/src/lib.rs', 'THIRD_PARTY_NOTICES.md',
                     '.github/workflows/ci.yml', 'tests/zsh/completion.test.zsh'):
            with self.subTest(path=path):
                result = self.select(path)
                self.assertEqual(result['scope'], 'FULL')
                self.assertIn('ALWAYS_ESCALATE', self.codes(result))

    def test_new_policy_sections_are_required(self):
        for key in ('audit_only', 'completion_assets'):
            with self.subTest(key=key):
                policy = {k: v for k, v in self.policy.items() if k != key}
                with self.assertRaises(self.mod.PlanError):
                    self.mod.select([], self.base, self.candidate, policy, self.registry)

    def test_completion_templates_need_one_binary_placeholder(self):
        for template in ('completions/zsh/_forge-cli', 'completions/{bin}/{bin}'):
            with self.subTest(template=template):
                policy = dict(self.policy, completion_assets=[template])
                with self.assertRaises(self.mod.PlanError):
                    self.mod.select([], self.base, self.candidate, policy, self.registry)


if __name__ == '__main__':
    unittest.main()
