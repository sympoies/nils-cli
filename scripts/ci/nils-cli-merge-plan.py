#!/usr/bin/env python3
"""Load the merge selector from an explicitly pinned trusted base commit."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import types

SELECTOR = 'scripts/ci/lib/merge_plan.py'
VERSION = 'nils-cli.merge-plan.v2'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path.cwd())
    parser.add_argument('--base', required=True, help='trusted target base B, full commit object ID')
    parser.add_argument('--head', required=True, help='reviewed head H, full commit object ID')
    parser.add_argument('--candidate', required=True, help='candidate merge M with ordered parents B,H')
    parser.add_argument('--tree', help='expected candidate tree T, full object ID (optional assertion)')
    args = parser.parse_args()
    inputs = {'base': None, 'head': None, 'candidate': None, 'tree': None}
    plan = {'schema_version': VERSION, 'status': 'PLAN_ERROR', 'action': 'HOLD', 'scope': None,
            'inputs': inputs, 'digests': {}, 'changes': [], 'selected_packages': [],
            'reasons': [], 'error': None}
    stage = 'inputs'
    code = 'INPUT_INVALID'

    def git(*arguments):
        return subprocess.run(['git', '--no-replace-objects', '-C', str(args.repo), *arguments], check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout

    try:
        for key in ('base', 'head', 'candidate'):
            value = getattr(args, key)
            if not re.fullmatch(r'(?:[0-9a-f]{40}|[0-9a-f]{64})', value):
                raise ValueError()
            if git('cat-file', '-t', value).strip() != b'commit':
                raise ValueError()
            inputs[key] = value
        code = 'CANDIDATE_MISMATCH'
        headers = git('cat-file', '-p', args.candidate).split(b'\n\n', 1)[0].splitlines()
        parents = [line[7:].decode('ascii') for line in headers if line.startswith(b'parent ')]
        if parents != [args.base, args.head]:
            raise ValueError()
        inputs['tree'] = git('rev-parse', args.candidate + '^{tree}').decode().strip()
        if args.tree is not None and inputs['tree'] != args.tree:
            raise ValueError()
        stage = 'selector'
        code = 'SELECTOR_FAILED'
        source = git('show', args.base + ':' + SELECTOR)
        selector_digest = hashlib.sha256(source).hexdigest()
        plan['digests']['selector'] = selector_digest
        module = types.ModuleType('trusted_merge_plan')
        # No candidate import or checkout-relative sys.path. The pinned source
        # itself owns every selection rule and its policy/registry reads.
        exec(compile(source, '<trusted-base-selector>', 'exec'), module.__dict__)
        plan = module.generate(args.repo, inputs, selector_digest)
    except Exception:
        # Publish fixed structured diagnostics, never exception/command text.
        plan['error'] = {'code': code, 'stage': stage, 'retryable': True,
                         'next_action': 'repair_plan_inputs', 'recovery': {}}
    plan['digests']['plan'] = hashlib.sha256(
        json.dumps(plan, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()).hexdigest()
    print(json.dumps(plan, sort_keys=True, separators=(',', ':'), ensure_ascii=True))
    return 0 if plan['status'] == 'READY' else 1


if __name__ == '__main__':
    raise SystemExit(main())
