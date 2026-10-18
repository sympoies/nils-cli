#!/usr/bin/env python3
"""Measure the same selected-package coverage corpus at immutable base and HEAD."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def coverage_counts(path):
    """Read complete, nonempty LCOV summaries without rounding percentages."""
    covered = found = 0
    records = 0
    source = None
    counts = {}
    for line in path.read_text().splitlines():
        if line.startswith('SF:'):
            if source is not None:
                raise ValueError('incomplete LCOV record')
            source, counts = line[3:], {}
        elif line.startswith(('LH:', 'LF:')):
            key, value = line.split(':', 1)
            if source is None or key in counts or not value.isdecimal():
                raise ValueError('invalid LCOV summary')
            counts[key] = int(value)
        elif line == 'end_of_record':
            if not source or set(counts) != {'LH', 'LF'} or counts['LH'] > counts['LF']:
                raise ValueError('invalid LCOV record')
            covered += counts['LH']
            found += counts['LF']
            records += 1
            source = None
    if source is not None or not records or found == 0:
        raise ValueError('empty or incomplete LCOV corpus')
    return {'covered': covered, 'lines': found}


def run_coverage(repo, output, arguments, env):
    command = ['cargo', 'llvm-cov', 'nextest', '--profile', 'ci', *arguments,
               '--no-fail-fast', '--lcov', '--output-path', str(output)]
    output.unlink(missing_ok=True)
    print('+ ' + ' '.join(command), flush=True)
    subprocess.run(command, cwd=repo, env=env, check=True)
    return coverage_counts(output)


def run(base, packages, excludes, output):
    if not re.fullmatch(r'(?:[0-9a-f]{40}|[0-9a-f]{64})', base):
        raise ValueError('an immutable coverage base is required')
    if not packages or any(not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]*', p) for p in packages):
        raise ValueError('invalid scoped coverage package selection')
    repo = Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], text=True).strip())
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
    subprocess.run(['git', 'cat-file', '-e', base + '^{commit}'], cwd=repo, check=True)
    arguments = []
    for package in packages:
        arguments += ['-p', package]
    arguments += ['--all-features']
    for name in excludes:
        arguments += ['--exclude-from-report', name]
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    evidence = repo / 'target/ci-merge-plan/scoped-coverage'
    evidence.mkdir(parents=True, exist_ok=True)
    receipt = {'schema_version': 'nils-cli.scoped-coverage-ratchet.v1',
               'base': base, 'candidate': head, 'packages': packages,
               'arguments': arguments, 'status': 'RUNNING', 'stage': 'baseline'}
    (evidence / 'base-lcov.info').unlink(missing_ok=True)
    (evidence / 'ratchet.json').write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    try:
        # A clone keeps Git-aware tests working without registering a worktree
        # or moving the candidate's refs. Baseline objects and profiles must not
        # share a target directory with the candidate: duplicate source mappings
        # otherwise corrupt the coverage corpus.
        with tempfile.TemporaryDirectory(prefix='ci-coverage-base-') as temporary:
            baseline = Path(temporary) / 'repository'
            subprocess.run(['git', 'clone', '--quiet', '--shared', '--no-checkout',
                            str(repo), str(baseline)], check=True)
            subprocess.run(['git', 'checkout', '--quiet', '--detach', base], cwd=baseline, check=True)
            baseline_env = dict(os.environ, CARGO_TARGET_DIR=str(Path(temporary) / 'target'))
            baseline_env.pop('CARGO_LLVM_COV_TARGET_DIR', None)
            baseline_env.pop('LLVM_PROFILE_FILE', None)
            receipt['baseline'] = run_coverage(baseline, output / 'base-lcov.info', arguments, baseline_env)
            shutil.copyfile(output / 'base-lcov.info', evidence / 'base-lcov.info')
            receipt['stage'] = 'candidate'
            receipt['current'] = run_coverage(repo, output / 'lcov.info', arguments, dict(os.environ))
        receipt['stage'] = 'compare'
        old, new = receipt['baseline'], receipt['current']
        passed = new['covered'] * old['lines'] >= old['covered'] * new['lines']
        receipt['status'] = 'PASS' if passed else 'FAIL'
        print(f'coverage ratchet: {receipt["status"]} '
              f'base={old["covered"]}/{old["lines"]} ({100 * old["covered"] / old["lines"]:.6f}%) '
              f'candidate={new["covered"]}/{new["lines"]} ({100 * new["covered"] / new["lines"]:.6f}%)',
              flush=True)
        return 0 if passed else 1
    finally:
        if receipt['status'] == 'RUNNING':
            receipt['status'] = 'FAIL'
        (evidence / 'ratchet.json').write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True)
    parser.add_argument('--output', type=Path, default=Path('target/coverage'))
    parser.add_argument('--exclude-from-report', action='append', default=[])
    parser.add_argument('packages', nargs='+')
    args = parser.parse_args()
    try:
        return run(args.base, args.packages, args.exclude_from_report, args.output)
    except subprocess.CalledProcessError as error:
        print(f'coverage ratchet: command failed (exit {error.returncode})', file=sys.stderr)
        return error.returncode if error.returncode > 0 else 1
    except (OSError, ValueError) as error:
        print(f'coverage ratchet: unavailable or invalid evidence: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
