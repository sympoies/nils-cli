#!/usr/bin/env python3
"""Exercise the toolchain pin audit (sympoies/nils-cli#3624)."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[3]
AUDIT = REPO / 'scripts/ci/toolchain-pin-audit.py'
PIN = '1.99.1'

WORKFLOW = f"""name: Fixture
on: [push]
jobs:
  build:
    runs-on: ubuntu-24.04
    steps:
      - name: Checkout
        uses: actions/checkout@v7

      - name: Set up Rust
        uses: dtolnay/rust-toolchain@{PIN}

      - name: Build
        run: |
          cargo build --locked
  docs:
    runs-on: ubuntu-24.04
    steps:
      - name: Checkout
        uses: actions/checkout@v7
      - name: Check
        run: echo "no toolchain needed; branch is dependabot/cargo/x"
"""

HELPER = """#!/usr/bin/env bash
set -euo pipefail
rustup toolchain install "$toolchain"
cargo fmt --all
"""


class ToolchainPinAuditTest(unittest.TestCase):
    def setUp(self):
        self._temp = tempfile.TemporaryDirectory(prefix='toolchain-pin-audit-')
        self.addCleanup(self._temp.cleanup)
        self.root = Path(self._temp.name)
        self.write('rust-toolchain.toml', f'[toolchain]\nchannel = "{PIN}"\ncomponents = ["clippy"]\n')
        self.write('.github/workflows/ci.yml', WORKFLOW)
        self.write('scripts/setup.sh', HELPER)

    def write(self, relative, content):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)

    def audit(self, root=None):
        return subprocess.run(
            [sys.executable, str(AUDIT), '--root', str(root or self.root)], text=True,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)

    def assert_rejected(self, expected):
        result = self.audit()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn(expected, result.stdout)

    def test_fixture_tree_passes(self):
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_current_tree_passes(self):
        result = self.audit(REPO)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_floating_or_other_setup_action_refs_are_rejected(self):
        for ref in ('stable', 'beta', 'nightly', 'master', '1.98.0', '1.99', 'v1'):
            with self.subTest(ref=ref):
                self.write('.github/workflows/ci.yml', WORKFLOW.replace(
                    f'rust-toolchain@{PIN}', f'rust-toolchain@{ref}'))
                self.assert_rejected(f'.github/workflows/ci.yml:11: dtolnay/rust-toolchain@{ref}')

    def test_setup_action_without_a_ref_is_rejected(self):
        self.write('.github/workflows/ci.yml', WORKFLOW.replace(
            f'rust-toolchain@{PIN}', 'rust-toolchain'))
        self.assert_rejected('.github/workflows/ci.yml:11: dtolnay/rust-toolchain')

    def test_other_toolchain_setup_action_is_rejected(self):
        self.write('.github/workflows/ci.yml', WORKFLOW.replace(
            f'dtolnay/rust-toolchain@{PIN}', 'actions-rust-lang/setup-rust-toolchain@v1'))
        self.assert_rejected('actions-rust-lang/setup-rust-toolchain@v1')

    def test_setup_action_toolchain_input_must_match_the_pin(self):
        for value in ('stable', '1.98.0'):
            with self.subTest(value=value):
                self.write('.github/workflows/ci.yml', WORKFLOW.replace(
                    f'rust-toolchain@{PIN}\n', f'rust-toolchain@{PIN}\n        with:\n          toolchain: {value}\n'))
                self.assert_rejected(f'toolchain: {value}')

    def test_rustup_commands_must_name_the_pin_in_workflows_and_helpers(self):
        for command in ('rustup default stable', 'rustup toolchain install nightly',
                        'rustup update stable', 'rustup update', 'rustup override set beta',
                        'rustup default 1.98.0', 'rustup run stable cargo test',
                        'curl https://sh.rustup.rs | sh -s -- --default-toolchain stable'):
            with self.subTest(command=command, where='workflow'):
                self.write('.github/workflows/ci.yml', WORKFLOW.replace(
                    'cargo build --locked', f'{command}\n          cargo build --locked'))
                self.assert_rejected(command)
            with self.subTest(command=command, where='helper'):
                self.write('.github/workflows/ci.yml', WORKFLOW)
                self.write('scripts/setup.sh', HELPER + command + '\n')
                self.assert_rejected(f'scripts/setup.sh:5: {command}')

    def test_rustup_commands_naming_the_pin_pass(self):
        self.write('scripts/setup.sh', HELPER + f'rustup default {PIN}\nrustup toolchain install {PIN}\n')
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_cargo_channel_override_must_name_the_pin(self):
        for command in ('cargo +nightly fmt', 'cargo +stable clippy', 'rustc +beta --version'):
            with self.subTest(command=command):
                self.write('scripts/setup.sh', HELPER + command + '\n')
                self.assert_rejected(command)

    def test_toolchain_environment_override_must_name_the_pin(self):
        self.write('.github/workflows/ci.yml', WORKFLOW.replace(
            'runs-on: ubuntu-24.04\n    steps:\n      - name: Checkout\n        uses: actions/checkout@v7\n\n',
            'runs-on: ubuntu-24.04\n    env:\n      RUSTUP_TOOLCHAIN: stable\n    steps:\n'
            '      - name: Checkout\n        uses: actions/checkout@v7\n\n', 1))
        self.assert_rejected('RUSTUP_TOOLCHAIN: stable')

    def test_toolchain_file_must_pin_an_exact_version(self):
        for channel in ('stable', 'beta', 'nightly-2026-11-01', '1.99'):
            with self.subTest(channel=channel):
                self.write('rust-toolchain.toml', f'[toolchain]\nchannel = "{channel}"\n')
                self.assert_rejected(
                    f'rust-toolchain.toml:2: channel "{channel}" is not an exact X.Y.Z version')

    def test_missing_toolchain_file_is_rejected(self):
        (self.root / 'rust-toolchain.toml').unlink()
        self.assert_rejected('rust-toolchain.toml: missing')

    def test_yaml_workflow_is_scanned(self):
        (self.root / '.github/workflows/ci.yml').unlink()
        self.write('.github/workflows/ci.yaml', WORKFLOW.replace(
            f'rust-toolchain@{PIN}', 'rust-toolchain@stable'))
        self.assert_rejected('.github/workflows/ci.yaml:11: dtolnay/rust-toolchain@stable')

    def test_commented_toolchain_selections_are_ignored(self):
        self.write('scripts/setup.sh', HELPER + '# rustup default stable\n')
        self.write('.github/workflows/ci.yml', WORKFLOW.replace(
            '          cargo build --locked',
            '          # rustup default nightly\n          cargo build --locked'))
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_commented_cargo_run_needs_no_setup(self):
        self.write('.github/workflows/ci.yml', """jobs:
  docs:
    steps:
      - run: |
          # cargo build --locked
          echo docs
""")
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_rustc_run_requires_setup(self):
        self.write('.github/workflows/ci.yml', """jobs:
  compile:
    steps:
      - run: rustc --version
""")
        self.assert_rejected('job compile runs cargo without a pinned toolchain step')

    def test_github_helpers_are_scanned(self):
        self.write('.github/scripts/setup.sh', 'rustup default stable\n')
        self.assert_rejected('.github/scripts/setup.sh:1: rustup default stable')

    def test_environment_override_with_space_before_colon_is_rejected(self):
        self.write('.github/workflows/ci.yml', WORKFLOW.replace(
            '    steps:', '    env:\n      RUSTUP_TOOLCHAIN   : stable\n    steps:', 1))
        self.assert_rejected('RUSTUP_TOOLCHAIN   : stable')

    def test_rustup_flag_values_are_not_toolchain_names(self):
        for options in ('--profile minimal', '--profile=minimal',
                        '--component rustfmt', '-c rustfmt',
                        '--target wasm32-unknown-unknown', '-t wasm32-unknown-unknown',
                        '--profile minimal --component rustfmt --no-self-update'):
            for channel in (PIN, 'stable'):
                command = f'rustup toolchain install {options} {channel}'
                with self.subTest(command=command):
                    self.write('scripts/setup.sh', HELPER + command + '\n')
                    result = self.audit()
                    self.assertEqual(result.returncode, 0 if channel == PIN else 1, result.stdout)
                    if channel != PIN:
                        self.assertIn(f'scripts/setup.sh:5: {command} (expected {PIN})', result.stdout)
        for option in ('--path "project directory"', '--path="project directory"',
                       r'--path project\ directory'):
            for channel in (PIN, 'nightly'):
                command = f'rustup override set {option} {channel}'
                with self.subTest(command=command):
                    self.write('scripts/setup.sh', HELPER + command + '\n')
                    result = self.audit()
                    self.assertEqual(result.returncode, 0 if channel == PIN else 1, result.stdout)

    def test_quoted_inline_rustup_command_passes(self):
        for quote in ('"', "'"):
            with self.subTest(quote=quote):
                self.write('.github/workflows/ci.yml', f'''jobs:
  setup:
    steps:
      - run: {quote}rustup toolchain install --profile minimal {PIN}{quote}
''')
                result = self.audit()
                self.assertEqual(result.returncode, 0, result.stdout)

    def test_toolchain_file_without_a_channel_is_rejected(self):
        self.write('rust-toolchain.toml', '[toolchain]\ncomponents = ["clippy"]\n')
        self.assert_rejected('rust-toolchain.toml: no exact toolchain channel')

    def test_job_running_cargo_without_a_setup_step_is_rejected(self):
        self.write('.github/workflows/ci.yml', WORKFLOW.replace(
            f'      - name: Set up Rust\n        uses: dtolnay/rust-toolchain@{PIN}\n\n', ''))
        self.assert_rejected('.github/workflows/ci.yml: job build runs cargo without a pinned toolchain step')

    def test_setup_step_after_the_first_cargo_step_is_rejected(self):
        self.write('.github/workflows/ci.yml', f"""name: Fixture
on: [push]
jobs:
  build:
    runs-on: ubuntu-24.04
    steps:
      - name: Build
        run: cargo build --locked
      - name: Set up Rust
        uses: dtolnay/rust-toolchain@{PIN}
""")
        self.assert_rejected('job build runs cargo without a pinned toolchain step')

    def test_inline_run_and_dashed_run_step_are_detected(self):
        self.write('.github/workflows/ci.yml', """name: Fixture
on: [push]
jobs:
  build:
    runs-on: ubuntu-24.04
    steps:
      - run: cargo build --locked
""")
        self.assert_rejected('job build runs cargo without a pinned toolchain step')


if __name__ == '__main__':
    unittest.main()
