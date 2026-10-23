#!/usr/bin/env python3
"""Compile and lint affected merge-group packages on macOS, including test targets."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

spec = importlib.util.spec_from_file_location(
    'merge_checks', Path(__file__).with_name('nils-cli-merge-plan-checks.py'))
merge_checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(merge_checks)


def run(repo, event_name, event, candidate, output):
    if event_name != 'merge_group':
        print('macOS compile/Clippy runs only for merge groups', flush=True)
        return 0
    expected, fallback = merge_checks.inputs(repo, event_name, event, candidate)
    plan = None
    if fallback is None:
        plan, fallback = merge_checks.make_plan(repo, expected)
    if plan is None:
        scope, packages = 'FULL', []
    else:
        scope, packages, fallback = merge_checks.plan_scope(plan, expected)
    # --all-targets includes integration tests and cfg(test) helpers without
    # executing tests. Full/unknown/empty scope must never become a no-op.
    command = ['cargo', 'clippy', '--locked', '--all-targets', '--all-features']
    if scope == 'SCOPED':
        for package in packages:
            command += ['-p', package]
    else:
        command += ['--workspace']
    command += ['--', '-D', 'warnings']
    output.mkdir(parents=True, exist_ok=True)
    if plan is not None:
        (output / 'plan.json').write_text(json.dumps(plan, sort_keys=True, indent=2) + '\n')
    receipt = {'schema_version': 'nils-cli.macos-clippy.v1', 'inputs': expected,
               'checkout': {'commit': candidate, 'tree': expected['tree']},
               'validation_scope': scope, 'packages': packages, 'fallback': fallback,
               'plan_digest': plan['digests']['plan'] if plan else None,
               'command': command, 'status': 'RUNNING'}
    receipt_path = output / 'validation.json'
    receipt_path.write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    print(f'macOS Clippy scope={scope} packages={",".join(packages)} fallback={fallback or "none"}', flush=True)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        merge_checks.write_summary(os.environ['GITHUB_STEP_SUMMARY'], scope, packages, fallback, plan)
    code = subprocess.run(command, cwd=repo).returncode
    receipt.update(status='PASS' if code == 0 else 'FAIL', exit_code=code)
    receipt_path.write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    return code


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path.cwd())
    parser.add_argument('--event-name', default=os.environ.get('GITHUB_EVENT_NAME'))
    parser.add_argument('--event-path', type=Path, default=os.environ.get('GITHUB_EVENT_PATH'))
    parser.add_argument('--candidate', default=os.environ.get('GITHUB_SHA'))
    parser.add_argument('--output', type=Path, default=Path('target/ci-macos-clippy'))
    args = parser.parse_args()
    try:
        event = json.loads(args.event_path.read_text())
        return run(args.repo.resolve(), args.event_name, event, args.candidate, args.output)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.CalledProcessError):
        print('error: macOS scope or event binding is invalid; validation held', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
