#!/usr/bin/env python3
"""Exercise local-fast package command ordering with mocked Cargo work."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


REPO = Path(__file__).resolve().parents[3]
LOCAL_FAST = REPO / 'scripts/ci/nils-cli-local-fast.sh'


def main():
    real_cargo = shutil.which('cargo')
    real_bash = shutil.which('bash')
    if not real_cargo or not real_bash:
        raise SystemExit('local-fast execution test requires cargo and bash')

    with tempfile.TemporaryDirectory(prefix='local-fast-execution-') as temp:
        root = Path(temp)
        bin_dir = root / 'bin'
        bin_dir.mkdir()
        log = root / 'cargo-commands.jsonl'
        cargo = bin_dir / 'cargo'
        cargo.write_text(
            '#!' + sys.executable + '\n'
            'import json, os, subprocess, sys\n'
            'args = sys.argv[1:]\n'
            'if args[:1] == ["metadata"]:\n'
            '    metadata = subprocess.check_output([\n'
            '        os.environ["LOCAL_FAST_TEST_REAL_CARGO"], "metadata",\n'
            '        "--no-deps", "--locked", "--all-features",\n'
            '        "--format-version", "1"], text=True)\n'
            '    result = json.loads(metadata)\n'
            '    result["resolve"] = {"nodes": []}\n'
            '    print(json.dumps(result))\n'
            '    raise SystemExit(0)\n'
            'with open(os.environ["LOCAL_FAST_TEST_CARGO_LOG"], "a") as out:\n'
            '    out.write(json.dumps(args) + "\\n")\n'
        )
        cargo.chmod(0o755)

        # The orchestration test stubs audit subprocesses and records Cargo
        # commands; metadata remains real so package/sibling discovery is real.
        fake_bash = bin_dir / 'bash'
        fake_bash.write_text('#!/bin/sh\nexit 0\n')
        fake_bash.chmod(0o755)
        fake_python = bin_dir / 'python3'
        fake_python.write_text(
            '#!/bin/sh\n'
            'if [ "${1-}" = "-" ]; then\n'
            f'  exec {sys.executable!s} "$@"\n'
            'fi\n'
            'exit 0\n'
        )
        fake_python.chmod(0o755)

        env = os.environ.copy()
        cargo_home = root / 'cargo-home'
        cargo_home.mkdir()
        env.update({
            'PATH': f'{bin_dir}{os.pathsep}{env.get("PATH", "")}',
            'CARGO_HOME': str(cargo_home),
            'BASH': real_bash,
            'CARGO': str(cargo),
            'LOCAL_FAST_TEST_REAL_CARGO': real_cargo,
            'LOCAL_FAST_TEST_CARGO_LOG': str(log),
            'NILS_CLI_GATE_ACTIVE': '1',
            'NILS_CLI_TEST_RUNNER': 'cargo',
        })
        result = subprocess.run(
            [real_bash, str(LOCAL_FAST),
             '--changed-file', 'crates/agent-session/src/lib.rs'],
            cwd=REPO, env=env, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, check=False,
        )
        if result.returncode:
            raise AssertionError(
                f'local-fast execution failed ({result.returncode}):\n{result.stdout}')

        commands = [json.loads(line) for line in log.read_text().splitlines()]
        build = ['build', '-p', 'nils-agent-session', '-p',
                 'nils-agent-hook', '--bins']
        test = ['test', '-p', 'nils-agent-session']
        if build not in commands:
            raise AssertionError(
                f'discovered-helper build was not executed: {commands!r}')
        if test not in commands:
            raise AssertionError(f'selected package tests did not run: {commands!r}')
        if commands.index(build) > commands.index(test):
            raise AssertionError(
                f'discovered-helper build ran after package tests: {commands!r}')

    print('PASS: local-fast builds discovered helper binaries before package tests')


if __name__ == '__main__':
    main()
