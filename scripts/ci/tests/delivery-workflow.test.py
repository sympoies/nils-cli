#!/usr/bin/env python3
"""Delivery DAG contracts and shell dispatch without compilers or hosted jobs."""
import importlib.util
from pathlib import Path
import re
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('ci_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


def workflow(name):
    return (ROOT / '.github/workflows' / name).read_text()


def job(text, name):
    return re.split(r'\n  [a-z_]+:\n', text.split(f'\n  {name}:\n', 1)[1], maxsplit=1)[0]


class DeliveryTests(unittest.TestCase):
    def test_release_queue_runs_shared_full_validation_before_required_linux_check(self):
        ci = workflow('ci.yml')
        self.assertNotIn('runs-on: macos', ci)
        self.assertNotIn('\n  test_macos:', ci)
        self.assertNotIn('deferred-full', ci)
        self.assertNotIn('\n  coverage:', ci)
        full = job(ci, 'full_validation')
        self.assertIn("github.event_name == 'schedule'", full)
        self.assertIn("github.event_name == 'workflow_dispatch'", full)
        self.assertIn("github.ref == 'refs/heads/main'", full)
        self.assertIn('uses: ./.github/workflows/full-validation.yml', full)
        self.assertIn('needs: changes', full)
        self.assertIn('always()', full)
        self.assertIn("github.event_name == 'merge_group'", full)
        self.assertIn("needs.changes.outputs.release_candidate == 'true'", full)
        self.assertIn("needs.changes.result == 'success'", full)
        linux = job(ci, 'test')
        self.assertIn('needs: [changes, full_validation]', linux)
        self.assertIn('FULL_VALIDATION_RESULT: ${{ needs.full_validation.result }}', linux)
        self.assertIn('Require release queue validation', linux)

    def test_pr_and_queue_consume_affected_plan(self):
        fixture = fixtures.WorkflowDispatchTests()
        for event in ('pull_request', 'merge_group'):
            self.assertIn('--ci-merge-plan', fixture.dispatch('test', event))

    def test_release_build_depends_on_full_validation_of_authorized_sha(self):
        release = workflow('release.yml')
        verify = job(release, 'verify_ci')
        full = job(release, 'full_validation')
        self.assertIn('validate-release-invocation.sh', verify)
        self.assertNotIn('runReleaseGate', verify)
        self.assertIn('needs: verify_ci', full)
        self.assertIn('uses: ./.github/workflows/full-validation.yml', full)
        self.assertIn('commit: ${{ github.sha }}', full)
        self.assertIn('ref: ${{ github.sha }}', job(release, 'build'))
        self.assertIn('reused: ${{ steps.reuse.outputs.reused }}', verify)
        self.assertIn('actions: read', verify)
        self.assertIn('findReusableValidation', verify)
        self.assertIn("if: needs.verify_ci.outputs.reused != 'true'", full)
        gate = job(release, 'validation_gate')
        self.assertIn('needs: [verify_ci, full_validation]', gate)
        self.assertIn('if: always()', gate)
        self.assertIn('requireReleaseValidation', gate)
        self.assertIn('needs: validation_gate', job(release, 'build'))
        self.assertIn('needs: build', job(release, 'release'))
        self.assertIn('needs: release', job(release, 'dispatch_homebrew_tap'))

    def test_shared_full_gate_checks_out_exact_sha_and_has_no_scope_shortcuts(self):
        full = workflow('full-validation.yml')
        self.assertIn('workflow_call:', full)
        self.assertNotIn('ref: ${{ inputs.commit }}', full)
        self.assertNotIn('--ci-merge-plan', full)
        self.assertNotIn('--docs-only', full)
        self.assertNotIn('release_only', full)
        self.assertNotIn('PR_DEFER_FULL', full)
        for name in ('test', 'test_containment', 'test_macos', 'cargo_deny', 'coverage', 'gate'):
            body = job(full, name)
            self.assertIn('ref: ${{ github.sha }}', body)
            self.assertIn('persist-credentials: false', body)
            self.assertIn('timeout-minutes:', body)
        macos = job(full, 'test_macos')
        self.assertIn('runs-on: macos-14', macos)
        self.assertIn('timeout-minutes: 75', macos)
        self.assertIn('NILS_CLI_COVERAGE_FAIL_UNDER_LINES: "85"', macos)
        self.assertIn('NILS_CLI_TEST_RUNNER: llvm-cov', macos)
        self.assertIn('test -s target/coverage/lcov.info', job(full, 'coverage'))
        gate = job(full, 'gate')
        self.assertIn('if: always()', gate)
        self.assertIn('needs: [test, test_macos, coverage, test_containment, cargo_deny]', gate)
        self.assertIn('requireFullValidation', gate)
        self.assertIn('toJSON(needs)', gate)
        self.assertIn('git rev-parse HEAD', gate)
        self.assertIn('EXPECTED_COMMIT: ${{ inputs.commit }}', gate)

    def test_full_checks_have_fragment_validator_prerequisites(self):
        full = workflow('full-validation.yml')
        self.assertIn('DEVLOG_LAYOUT: fragments', full)
        for name in ('test', 'test_macos'):
            body = job(full, name)
            self.assertIn('cargo build --locked -p nils-devlog', body)
            self.assertLess(body.index('Build devlog validator'), body.index('Nils CLI checks'))

    def test_validator_setup_exposes_only_devlog_to_later_steps(self):
        setups = (('ci.yml', 'test', 'Build repository devlog checker'),
                  ('full-validation.yml', 'test', 'Build devlog validator'),
                  ('full-validation.yml', 'test_macos', 'Build devlog validator'))
        for filename, name, step_name in setups:
            with self.subTest(workflow=filename, job=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                binaries = root / 'target/debug'
                binaries.mkdir(parents=True)
                for binary in ('devlog', 'semantic-commit', 'agent-hook'):
                    (binaries / binary).write_text(binary + '\n')
                tools = root / 'tools'
                tools.mkdir()
                cargo = tools / 'cargo'
                cargo.write_text('#!/bin/sh\nexit 0\n')
                cargo.chmod(0o755)
                runner_temp = root / 'runner-temp'
                runner_temp.mkdir()
                github_path = root / 'github-path'
                body = job(workflow(filename), name)
                step = body.split(f'      - name: {step_name}\n', 1)[1].split('\n      - name:', 1)[0]
                script = step.split('        run: |\n', 1)[1]
                script = '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))
                env = dict(os.environ, RUNNER_TEMP=str(runner_temp), GITHUB_PATH=str(github_path),
                           GITHUB_WORKSPACE=str(root), PATH=str(tools) + os.pathsep + os.environ['PATH'])
                result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script], cwd=root,
                                        env=env, text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                additions = github_path.read_text().splitlines()
                self.assertEqual(len(additions), 1)
                exposed = Path(additions[0])
                self.assertEqual(sorted(path.name for path in exposed.iterdir()), ['devlog'])
                self.assertEqual(exposed.parent, runner_temp)
                self.assertEqual((exposed / 'devlog').read_text(), 'devlog\n')

    def test_required_linux_checks_fail_closed(self):
        ci = workflow('ci.yml')
        for name in ('test', 'cargo_deny', 'test_containment'):
            condition = next(line.removeprefix('    if: ') for line in job(ci, name).splitlines()
                             if line.startswith('    if: '))
            for event in ('pull_request', 'merge_group', 'push', 'schedule', 'workflow_dispatch'):
                with self.subTest(job=name, event=event):
                    expression = condition.replace('github.event_name', repr(event)).replace('always()', 'True')
                    observed = eval(expression.replace('&&', 'and').replace('||', 'or'),
                                    {'__builtins__': {}}, {})
                    self.assertEqual(observed, event in ('pull_request', 'merge_group'))

    def test_failed_change_detection_cannot_be_accepted_as_a_skipped_test(self):
        body = job(workflow('ci.yml'), 'test')
        step = body.split('      - name: Require change detection\n', 1)[1]
        script = step.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
        script = '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))
        for state in ('success', 'failure', 'cancelled', 'skipped', ''):
            result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script],
                env=dict(os.environ, CHANGES_RESULT=state), capture_output=True)
            self.assertEqual(result.returncode == 0, state == 'success')

    def test_release_queue_failures_block_the_required_linux_check(self):
        body = job(workflow('ci.yml'), 'test')
        step = body.split('      - name: Require release queue validation\n', 1)[1]
        script = step.split('        run: |\n', 1)[1].split('\n      - name:', 1)[0]
        script = '\n'.join(line[10:] for line in script.splitlines() if line.startswith('          '))
        for event in ('pull_request', 'merge_group'):
            for candidate in ('true', 'false'):
                for state in ('success', 'failure', 'cancelled', 'skipped', ''):
                    with self.subTest(event=event, candidate=candidate, state=state):
                        result = subprocess.run(['bash', '-euo', 'pipefail', '-c', script],
                            env=dict(os.environ, EVENT_NAME=event, RELEASE_CANDIDATE=candidate,
                                     FULL_VALIDATION_RESULT=state), capture_output=True)
                        allowed = event != 'merge_group' or candidate != 'true' or state == 'success'
                        self.assertEqual(result.returncode == 0, allowed)

    def test_full_workflow_runs_for_release_queues_and_nightly_even_when_detection_skips(self):
        full = job(workflow('ci.yml'), 'full_validation')
        condition = next(line.removeprefix('    if: ') for line in full.splitlines()
                         if line.startswith('    if: '))
        for event in ('pull_request', 'merge_group', 'push', 'schedule', 'workflow_dispatch'):
            for candidate in ('true', 'false'):
                for detection in ('success', 'failure', 'cancelled', 'skipped'):
                    expression = condition.replace('github.event_name', repr(event))
                    expression = expression.replace('github.ref', repr('refs/heads/main'))
                    expression = expression.replace('needs.changes.result', repr(detection))
                    expression = expression.replace('needs.changes.outputs.release_candidate', repr(candidate))
                    observed = eval(expression.replace('always()', 'True').replace('&&', 'and').replace('||', 'or'),
                                    {'__builtins__': {}}, {})
                    expected = event in ('schedule', 'workflow_dispatch') or (
                        event == 'merge_group' and candidate == 'true' and detection == 'success')
                    self.assertEqual(observed, expected)

    def test_nightly_report_serializes_tracker_updates_with_least_permissions(self):
        ci = workflow('ci.yml')
        report = job(ci, 'nightly_report')
        self.assertIn('always()', report)
        self.assertIn('needs: full_validation', report)
        permissions = re.search(r'^    permissions:\n((?:      [^\n]+\n)+)', report, re.M).group(1)
        self.assertEqual(dict(line.strip().split(': ', 1) for line in permissions.splitlines()),
                         {'contents': 'read', 'actions': 'read', 'issues': 'write'})
        self.assertIn('persist-credentials: false', report)
        self.assertIn('trackNightly', report)
        self.assertIn("'nightly-main'", ci)
        self.assertIn("cancel-in-progress: ${{ github.event_name == 'pull_request' }}", ci)
        self.assertIn('node scripts/ci/tests/nightly-tracker.test.cjs', (ROOT / '.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh').read_text())


if __name__ == '__main__':
    unittest.main(verbosity=2)
