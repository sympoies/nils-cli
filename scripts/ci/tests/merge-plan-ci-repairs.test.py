#!/usr/bin/env python3
"""Offline completion freshness and authoritative CI output regressions."""
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('ci_fixtures', Path(__file__).with_name('merge-plan-ci.test.py'))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


def runner_outputs(text):
    """Parse the scalar/multiline forms used by the runner's output command."""
    rows, result, index = text.splitlines(), {}, 0
    while index < len(rows):
        line = rows[index]
        index += 1
        if not line:
            continue
        if '=' in line and ('<<' not in line or line.index('=') < line.index('<<')):
            key, value = line.split('=', 1)
        else:
            key, delimiter = line.split('<<', 1)
            values = []
            while rows[index] != delimiter:
                values.append(rows[index])
                index += 1
            index += 1
            value = '\n'.join(values)
        result[key] = value
    return result


class RepairTests(unittest.TestCase):
    git = fixtures.ConsumerExecutionTests.git
    candidate = fixtures.ConsumerExecutionTests.candidate
    event = fixtures.ConsumerExecutionTests.event
    execute = fixtures.ConsumerExecutionTests.execute
    receipt = fixtures.ConsumerExecutionTests.receipt

    def setUp(self):
        fixtures.ConsumerExecutionTests.setUp(self)
        self.env['CARGO_TARGET_DIR'] = str(self.root / 'target')
        for path in ('scripts/ci/completion-freshness-audit.sh', 'scripts/ci/lib/cargo-target-dir.sh'):
            dest = self.repo / path
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes((ROOT / path).read_bytes())
        matrix = self.repo / 'docs/specs/completion-coverage-matrix-v1.md'
        matrix.parent.mkdir(parents=True)
        matrix.write_text('| `api-websocket` | `required` | present | present | none | static | fixture |\n'
                          '| `unselected-cli` | `required` | present | present | none | static | fixture |\n')
        for shell, name in [('bash', 'api-websocket'), ('zsh', '_api-websocket')]:
            asset = self.repo / f'completions/{shell}/{name}'
            asset.parent.mkdir(parents=True)
            asset.write_text(f'fixture {shell} --old-flag\n')
        (self.bin / 'cargo').rename(self.bin / 'cargo-base')
        cargo = self.bin / 'cargo'
        cargo.write_text('''#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
args = sys.argv[1:]
base = str(pathlib.Path(__file__).with_name('cargo-base'))
if args[0] == 'metadata':
    result = subprocess.run([base, *args], capture_output=True, text=True)
    if result.returncode:
        raise SystemExit(result.returncode)
    data = json.loads(result.stdout)
    if os.environ.get('FIXTURE_CLI'):
        for package in data['packages']:
            if package['name'] in ('selected', 'unrelated'):
                name = 'api-websocket' if package['name'] == 'selected' else 'unselected-cli'
                package['targets'] = [{'name': name, 'kind': ['bin']}]
    print(json.dumps(data))
    raise SystemExit(0)
if args[0] == 'nextest':
    path = os.environ.get('GITHUB_OUTPUT') or os.environ.get('FIXTURE_OUTPUT_PATH')
    if os.environ.get('FORGE_SCOPE_OUTPUT') and path:
        with open(path, 'a') as stream:
            stream.write('validation_scope<<validation_scope=SCOPED\\nFULL\\n')
    pathlib.Path(os.environ['FAKE_CARGO_LOG'] + '.output-env').write_text(
        str('GITHUB_OUTPUT' in os.environ))
result = subprocess.run([base, *args])
if result.returncode:
    raise SystemExit(result.returncode)
if args[0] == 'build' and os.environ.get('FIXTURE_CLI'):
    binary = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / 'debug/api-websocket'
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text('#!/usr/bin/env bash\\nprintf "fixture %s --new-flag\\\\n" "$2"\\n')
    binary.chmod(0o755)
''')
        cargo.chmod(0o755)

    def output_text(self):
        path = Path(self.env['GITHUB_OUTPUT'])
        return path.read_text() if path.exists() else ''

    def test_selected_cli_stale_completions_fail_before_tests(self):
        self.candidate()
        result, rows = self.execute(env={'FIXTURE_CLI': '1'})
        self.assertNotEqual(result.returncode, 0, 'stale committed assets passed the scoped lane')
        self.assertIn('stale bash completion asset', result.stdout)
        self.assertIn('stale zsh completion asset', result.stdout)
        self.assertFalse(any(row.get('args', [''])[0] == 'nextest' for row in rows))
        self.assertEqual(self.receipt()['status'], 'FAIL')
        self.assertEqual(self.output_text(), '')

    def test_regenerated_selected_assets_pass_without_unselected_binary_build(self):
        for shell, name in [('bash', 'api-websocket'), ('zsh', '_api-websocket')]:
            (self.repo / f'completions/{shell}/{name}').write_text(f'fixture {shell} --new-flag\n')
        self.candidate()
        result, rows = self.execute(env={'FIXTURE_CLI': '1'})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PASS: completion freshness audit (required=1, snapshots_checked=2', result.stdout)
        builds = [row['args'] for row in rows if row.get('args', [''])[0] == 'build']
        self.assertEqual(len(builds), 1)
        self.assertNotIn('--workspace', builds[0])
        self.assertFalse((Path(self.env['CARGO_TARGET_DIR']) / 'debug/unselected-cli').exists())

    def test_unavailable_completion_metadata_fails_closed(self):
        self.candidate()
        result, _ = self.execute(env={'FIXTURE_CLI': '1', 'FAIL_REPORT_METADATA': '1'})
        self.assertNotEqual(result.returncode, 0, 'unavailable owning metadata passed freshness coverage')
        self.assertEqual(self.output_text(), '')

    def test_inherited_multiline_output_cannot_forge_full_scope(self):
        self.candidate()
        result, _ = self.execute(env={'FORGE_SCOPE_OUTPUT': '1'})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(runner_outputs(self.output_text()), {'validation_scope': 'SCOPED'})
        self.assertEqual(self.receipt()['validation_scope'], 'SCOPED')
        self.assertEqual(Path(str(self.log) + '.output-env').read_text(), 'False')

    def test_executor_known_output_path_cannot_frame_authoritative_scope(self):
        self.candidate()
        result, _ = self.execute(env={'FORGE_SCOPE_OUTPUT': '1',
                                     'FIXTURE_OUTPUT_PATH': self.env['GITHUB_OUTPUT']})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output_text(), 'validation_scope=SCOPED\n')
        self.assertEqual(runner_outputs(self.output_text()), {'validation_scope': 'SCOPED'})

    def test_failed_executor_cannot_leave_forged_scope_output(self):
        self.candidate()
        result, _ = self.execute(env={'FORGE_SCOPE_OUTPUT': '1', 'FAIL_CARGO_COMMAND': 'nextest',
                                     'FIXTURE_OUTPUT_PATH': self.env['GITHUB_OUTPUT']})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.receipt()['status'], 'FAIL')
        self.assertEqual(self.output_text(), '')


if __name__ == '__main__':
    unittest.main(verbosity=2)
