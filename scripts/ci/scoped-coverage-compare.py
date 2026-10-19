#!/usr/bin/env python3
"""Join exact-snapshot selected coverage measurements without running tests."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sys

spec = importlib.util.spec_from_file_location('scoped_ratchet', Path(__file__).with_name('scoped-coverage-ratchet.py'))
ratchet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ratchet)
OID = re.compile(r'(?:[0-9a-f]{40}|[0-9a-f]{64})\Z')


def measurement(directory, side, base, candidate):
    receipt = json.loads((directory / 'ratchet.json').read_text())
    expected = {'schema_version': 'nils-cli.scoped-coverage-measurement.v1',
                'side': side, 'base': base, 'candidate': candidate, 'status': 'MEASURED'}
    if any(receipt.get(key) != value for key, value in expected.items()):
        raise ValueError(f'{side} measurement is failed, stale or invalid')
    key, filename = ('baseline', 'base-lcov.info') if side == 'base' else ('current', 'lcov.info')
    counts = ratchet.coverage_counts(directory / filename)
    if counts != receipt.get(key):
        raise ValueError(f'{side} LCOV disagrees with measurement')
    packages = receipt.get('packages')
    if (not isinstance(packages, list) or not packages
            or any(not isinstance(p, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]*', p) for p in packages)
            or len(set(packages)) != len(packages)):
        raise ValueError(f'{side} package corpus is invalid')
    arguments = receipt.get('arguments')
    prefix = [arg for package in packages for arg in ('-p', package)] + ['--all-features']
    if (not isinstance(arguments, list) or arguments[:len(prefix)] != prefix
            or len(arguments[len(prefix):]) % 2
            or any(arguments[i] != '--exclude-from-report'
                   for i in range(len(prefix), len(arguments), 2))):
        raise ValueError(f'{side} command corpus is invalid')
    if not isinstance(receipt.get('coverage_identity'), str) or not receipt['coverage_identity']:
        raise ValueError(f'{side} coverage runtime identity is missing')
    return receipt, counts


def compare(base, candidate, baseline, current, output):
    if not OID.fullmatch(base) or not OID.fullmatch(candidate):
        raise ValueError('immutable base and candidate are required')
    old, old_counts = measurement(baseline, 'base', base, candidate)
    new, new_counts = measurement(current, 'candidate', base, candidate)
    for key in ('packages', 'arguments', 'coverage_identity'):
        if old[key] != new[key]:
            raise ValueError(f'measurement {key} mismatch')
    passed = ratchet.compare_counts(old_counts, new_counts)
    result = {'schema_version': 'nils-cli.scoped-coverage-ratchet.v1',
              'base': base, 'candidate': candidate, 'packages': new['packages'],
              'arguments': new['arguments'], 'coverage_identity': new['coverage_identity'],
              'baseline': old_counts, 'current': new_counts, 'stage': 'compare',
              'status': 'PASS' if passed else 'FAIL'}
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    shutil.copyfile(baseline / 'base-lcov.info', output.parent / 'base-lcov.info')
    message = (f"coverage ratchet: {result['status']} base={old_counts['covered']}/{old_counts['lines']} "
               f"candidate={new_counts['covered']}/{new_counts['lines']} (parallel selected measurements)")
    print(message, flush=True)
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a') as stream:
            stream.write(f'Base `{base}`; candidate `{candidate}`. {message}\n')
    return 0 if passed else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True)
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--current', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--validation-receipt', type=Path)
    args = parser.parse_args()
    # Never retain a previous comparison as evidence for an invalid join.
    args.output.unlink(missing_ok=True)
    returncode = 1
    try:
        returncode = compare(args.base, args.candidate, args.baseline, args.current, args.output)
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f'coverage ratchet: invalid parallel evidence: {error}', file=sys.stderr)
    finally:
        if args.validation_receipt:
            receipt = json.loads(args.validation_receipt.read_text())
            receipt.update(status='PASS' if returncode == 0 else 'FAIL',
                           exit_code=returncode, coverage_execution='parallel')
            args.validation_receipt.write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    return returncode


if __name__ == '__main__':
    raise SystemExit(main())
