#!/usr/bin/env python3
"""Consume the trusted-base merge plan for the checked-out CI candidate."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

LAUNCHER = 'scripts/ci/nils-cli-merge-plan.py'
OID = re.compile(r'(?:[0-9a-f]{40}|[0-9a-f]{64})\Z')


def git(repo, *args, timeout=None):
    return subprocess.check_output(['git', '--no-replace-objects', '-C', str(repo), *args],
                                   stderr=subprocess.DEVNULL, timeout=timeout).decode().strip()


def squash_candidate(repo, base, candidate, tree, head_ref):
    """Represent a proven single-PR squash tree without changing the checkout."""
    match = re.fullmatch(r'refs/heads/gh-readonly-queue/.+/pr-([1-9][0-9]*)-([0-9a-f]+)', head_ref)
    if match is None or match[2] != base:
        return None, None, 'squash_provenance_unavailable'
    try:
        ref = f'refs/pull/{match[1]}/head'
        remote = git(repo, 'ls-remote', '--exit-code', 'origin', ref, timeout=60).split()
        if len(remote) != 2 or remote[1] != ref or not OID.fullmatch(remote[0]):
            raise ValueError()
        head = remote[0]
        git(repo, 'fetch', '--no-write-fetch-head', 'origin', head, timeout=60)
        # Local merge machinery must not select candidate programs. Attributes
        # or configured external merge drivers use the full fallback instead.
        config = subprocess.run(['git', '-C', str(repo), 'config', '--name-only', '--get-regexp',
                                 r'^merge\..*\.driver$|^merge\.default$'],
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if config.returncode not in (0, 1) or config.stdout:
            return head, None, 'squash_merge_unsupported'
        for revision in (base, head, candidate):
            paths = git(repo, 'ls-tree', '-r', '--name-only', '-z', revision).split('\0')
            if any(path.rsplit('/', 1)[-1].casefold() == '.gitattributes' for path in paths):
                return head, None, 'squash_merge_unsupported'
        merged_tree = git(repo, 'merge-tree', '--write-tree', base, head).splitlines()[0]
        if merged_tree != tree:
            return head, None, 'squash_tree_mismatch'
        # This is an unreferenced planning object, never a source commit or a
        # replacement checkout. Its tree is the actual queue commit's tree.
        env = dict(os.environ, GIT_AUTHOR_NAME='CI plan', GIT_COMMITTER_NAME='CI plan',
                   GIT_AUTHOR_EMAIL='ci-plan@example.invalid', GIT_COMMITTER_EMAIL='ci-plan@example.invalid',
                   GIT_AUTHOR_DATE='@0 +0000', GIT_COMMITTER_DATE='@0 +0000')
        result = subprocess.run(['git', '--no-replace-objects', '-C', str(repo), 'commit-tree',
                                 tree, '-p', base, '-p', head],
                                input=f'CI planning representation of {candidate}\n', text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=env, check=True)
        return head, result.stdout.strip(), None
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError, ValueError, IndexError):
        return None, None, 'squash_provenance_unavailable'


def inputs(repo, event_name, event, candidate):
    if not isinstance(candidate, str) or not OID.fullmatch(candidate):
        raise ValueError('invalid candidate')
    parents = git(repo, 'show', '-s', '--format=%P', candidate).split()
    if event_name == 'pull_request':
        event_base = event['pull_request']['base']['sha']
        if not isinstance(event_base, str) or not OID.fullmatch(event_base) or event_base == candidate:
            raise ValueError('invalid event base')
        # Checkout fetches the immutable context SHA. PR event base metadata
        # can lag that platform-generated merge; its first parent is the base
        # actually tested. The reviewed event head must still be parent two.
        base = parents[0] if len(parents) == 2 else None
        head = event['pull_request']['head']['sha']
    elif event_name == 'merge_group':
        base = event['merge_group']['base_sha']
        if event['merge_group']['head_sha'] != candidate:
            raise ValueError('merge-group candidate mismatch')
        head = parents[1] if len(parents) == 2 else None
    else:
        raise ValueError('unsupported event')
    if not isinstance(base, str) or not OID.fullmatch(base):
        raise ValueError('invalid event object IDs')
    squash = event_name == 'merge_group' and parents == [base]
    if (not squash and (not isinstance(head, str) or not OID.fullmatch(head) or parents != [base, head])):
        raise ValueError('candidate parent mismatch')
    if git(repo, 'rev-parse', 'HEAD') != candidate:
        raise ValueError('candidate parents or checkout mismatch')
    for args in (('diff', '--quiet'), ('diff', '--cached', '--quiet')):
        subprocess.run(['git', '--no-replace-objects', '-C', str(repo), *args], check=True)
    tree = git(repo, 'rev-parse', candidate + '^{tree}')
    fallback = None
    planning_candidate = candidate
    if squash:
        head, adapted, fallback = squash_candidate(repo, base, candidate, tree,
                                                   event['merge_group'].get('head_ref', ''))
        if adapted is not None:
            planning_candidate = adapted
    return {'base': base, 'head': head, 'candidate': planning_candidate, 'tree': tree}, fallback


def plan_scope(plan, expected):
    if plan.get('schema_version') not in ('nils-cli.merge-plan.v1', 'nils-cli.merge-plan.v2') or plan.get('inputs') != expected:
        raise ValueError('plan version or inputs mismatch')
    digests = plan.get('digests', {})
    unsigned = dict(plan, digests={k: v for k, v in digests.items() if k != 'plan'})
    digest = hashlib.sha256(json.dumps(unsigned, sort_keys=True, separators=(',', ':'),
                                      ensure_ascii=True).encode()).hexdigest()
    if digests.get('plan') != digest:
        raise ValueError('plan digest mismatch')
    if plan.get('status') == 'PLAN_ERROR' and plan.get('action') == 'HOLD':
        if plan.get('scope') is not None or plan.get('selected_packages') != [] or not plan.get('error'):
            raise ValueError('invalid HOLD plan')
        return 'FULL', [], 'planner_hold'
    if (plan.get('status') != 'READY' or plan.get('action') != 'VALIDATE'
            or plan.get('error') is not None or plan.get('scope') not in ('SCOPED', 'FULL', 'NONE')):
        raise ValueError('invalid usable plan')
    required = ('selector', 'policy', 'registry', 'schema', 'diff', 'base_metadata', 'candidate_metadata')
    if any(not re.fullmatch(r'[0-9a-f]{64}', digests.get(key, '')) for key in required):
        raise ValueError('missing plan evidence digest')
    packages = plan.get('selected_packages')
    if not isinstance(packages, list):
        raise ValueError('invalid package selection')
    names = []
    for package in packages:
        name = package.get('name')
        manifest = package.get('manifest_path', '')
        if (not isinstance(name, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]*', name)
                or not package.get('id') or not isinstance(manifest, str)
                or manifest.startswith('/') or any(part in ('', '.', '..') for part in manifest.split('/'))):
            raise ValueError('invalid selected package')
        names.append(name)
    if len(names) != len(set(names)):
        raise ValueError('duplicate selected package')
    # Empty/NONE is not a validation PASS. The established docs/release routes
    # run before this code lane; any remaining empty selection gets full checks.
    if plan['scope'] != 'SCOPED' or not names:
        return 'FULL', [], 'empty_selection' if not names else None
    return 'SCOPED', sorted(names), None


def make_plan(repo, expected):
    try:
        source = subprocess.check_output(['git', '--no-replace-objects', '-C', str(repo),
                                          'show', expected['base'] + ':' + LAUNCHER],
                                         stderr=subprocess.DEVNULL)
    except subprocess.CalledProcessError:
        # Initial rollout cannot run a consumer absent from B. Keep the old
        # full gate; never load a candidate selector or invent a READY receipt.
        return None, 'trusted_base_launcher_missing'
    with tempfile.TemporaryDirectory(prefix='ci-trusted-plan-') as directory:
        launcher = Path(directory) / 'launcher.py'
        launcher.write_bytes(source)
        result = subprocess.run([sys.executable, str(launcher), '--repo', str(repo),
                                 '--base', expected['base'], '--head', expected['head'],
                                 '--candidate', expected['candidate'], '--tree', expected['tree']],
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        plan = json.loads(result.stdout)
    except ValueError:
        raise ValueError('planner output is not JSON') from None
    if result.returncode not in (0, 1):
        raise ValueError('planner process failed')
    if result.returncode != (0 if plan.get('status') == 'READY' else 1):
        raise ValueError('planner status disagrees with process result')
    return plan, None


def run(repo, event_name, event, candidate, output, bash='bash', plan_only=False):
    if plan_only and event_name != 'pull_request':
        raise ValueError('scope preview is only available for PRs')
    expected, fallback = inputs(repo, event_name, event, candidate)
    plan = None
    if fallback is None:
        plan, fallback = make_plan(repo, expected)
    if plan is None:
        scope, packages = 'FULL', []
    else:
        scope, packages, fallback = plan_scope(plan, expected)
    output.mkdir(parents=True, exist_ok=True)
    if plan is not None:
        (output / 'plan.json').write_text(json.dumps(plan, sort_keys=True, separators=(',', ':')) + '\n')
    if plan_only:
        summary = {'schema_version': 'nils-cli.merge-planning.v1', 'status': 'PLANNED',
                   'inputs': expected, 'validation_scope': scope, 'packages': packages,
                   'fallback': fallback, 'plan_digest': plan['digests']['plan'] if plan else None}
        (output / 'planning.json').write_text(json.dumps(summary, sort_keys=True, indent=2) + '\n')
        if os.environ.get('GITHUB_OUTPUT'):
            Path(os.environ['GITHUB_OUTPUT']).write_text(f'validation_scope={scope}\n')
        print(f'PR scope preview={scope}; no validation executed', flush=True)
        return 0
    command = [bash]
    if scope == 'SCOPED':
        command += ['scripts/ci/nils-cli-scoped-checks.sh', *packages]
    else:
        command += ['scripts/ci/nils-cli-checks-entrypoint.sh']
    runner = os.environ.get('NILS_CLI_TEST_RUNNER', 'nextest')
    ratchet = scope == 'SCOPED' and runner == 'llvm-cov'
    receipt = {'schema_version': 'nils-cli.merge-validation.v1', 'inputs': expected,
               'checkout': {'commit': candidate, 'tree': expected['tree']},
               'event_base': event['pull_request']['base']['sha'] if event_name == 'pull_request'
                             else event['merge_group']['base_sha'],
               'plan_digest': plan['digests']['plan'] if plan is not None else None,
               'validation_scope': scope, 'packages': packages, 'fallback': fallback,
               'test_runner': runner,
               'coverage_policy': 'base-no-decrease' if ratchet else ('absolute-floor' if runner == 'llvm-cov' else None),
               'coverage_base': expected['base'] if ratchet else None,
               'coverage_fail_under_lines': None if ratchet else os.environ.get('NILS_CLI_COVERAGE_FAIL_UNDER_LINES', '85'),
               'doctests': os.environ.get('NILS_CLI_SKIP_DOCTESTS', '0') != '1',
               'command': command, 'status': 'RUNNING'}
    deferred = (event_name == 'pull_request' and runner == 'llvm-cov' and scope == 'FULL'
                and os.environ.get('NILS_CLI_PR_DEFER_FULL') == '1')
    if deferred:
        scope = 'DEFERRED_FULL'
        receipt.update(schema_version='nils-cli.merge-validation.v2',
                       validation_scope=scope, command=[], status='DEFERRED',
                       coverage_policy=None, coverage_fail_under_lines=None,
                       deferred_reason='deferred-full: runs in merge group')
    (output / 'validation.json').write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    print(f'CI snapshot base={expected["base"]} event_base={receipt["event_base"]} '
          f'head={expected["head"]} candidate={candidate}', flush=True)
    print(f'CI validation scope={scope} packages={",".join(packages)} fallback={fallback or "none"}', flush=True)
    github_output = os.environ.get('GITHUB_OUTPUT')
    executor_env = dict(os.environ)
    executor_env.pop('GITHUB_OUTPUT', None)
    executor_env['NILS_CLI_SCOPED_COVERAGE_BASE'] = expected['base']
    if deferred:
        print('deferred-full: runs in merge group; no full macOS tests or coverage executed on this PR', flush=True)
        returncode = 0
    else:
        returncode = subprocess.run(command, cwd=repo, env=executor_env).returncode
        receipt.update(status='PASS' if returncode == 0 else 'FAIL')
    receipt['exit_code'] = returncode
    (output / 'validation.json').write_text(json.dumps(receipt, sort_keys=True, indent=2) + '\n')
    if github_output and (returncode == 0 or Path(github_output).exists()):
        # This consumer owns the step's sole output. Replace executor-written
        # framing rather than appending a delimiter that could turn SCOPED
        # into a forged multiline FULL value; failed checks emit no scope.
        with open(github_output, 'w') as stream:
            if returncode == 0:
                stream.write(f'validation_scope={scope}\n')
    return returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path.cwd())
    parser.add_argument('--event-name', default=os.environ.get('GITHUB_EVENT_NAME'))
    parser.add_argument('--event-path', type=Path, default=os.environ.get('GITHUB_EVENT_PATH'))
    parser.add_argument('--candidate', default=os.environ.get('GITHUB_SHA'))
    parser.add_argument('--output', type=Path, default=Path('target/ci-merge-plan'))
    parser.add_argument('--bash', default=os.environ.get('BASH', 'bash'))
    parser.add_argument('--plan-only', action='store_true', help='preview trusted PR scope without executing validation')
    args = parser.parse_args()
    try:
        event = json.loads(args.event_path.read_text())
        return run(args.repo.resolve(), args.event_name, event, args.candidate, args.output, args.bash, args.plan_only)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.CalledProcessError):
        print('error: CI merge plan or event binding is invalid; validation held', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
