"""Trusted-base merge-plan v1 selector. This module never runs validation gates."""
import fnmatch
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tempfile

VERSION = 'nils-cli.merge-plan.v1'
POLICY_PATH = 'scripts/ci/merge-plan-policy.v1.json'
REGISTRY_PATH = 'scripts/ci/merge-plan-extra-inputs.v1.json'
SCHEMA_PATH = 'scripts/ci/merge-plan.schema.v1.json'


class PlanError(Exception):
    def __init__(self, code, stage):
        self.code = code
        self.stage = stage


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def valid_path(path):
    return (isinstance(path, str) and bool(path) and not path.startswith('/')
            and all(part not in ('', '.', '..') for part in path.split('/')))


def parse_diff(raw):
    """Parse git name-status -z, retaining both names of copies and renames."""
    if raw and not raw.endswith(b'\0'):
        raise PlanError('DIFF_FAILED', 'diff')
    fields = raw.split(b'\0')[:-1]
    changes = []
    while fields:
        status = fields.pop(0).decode('ascii', errors='replace')
        if status in ('A', 'D', 'M', 'T'):
            count = 1
        elif status[:1] in ('R', 'C') and status[1:].isdigit() and int(status[1:]) <= 100:
            count = 2
        else:
            raise PlanError('DIFF_FAILED', 'diff')
        if len(fields) < count:
            raise PlanError('DIFF_FAILED', 'diff')
        paths = [fields.pop(0).decode('utf-8', errors='surrogateescape') for _ in range(count)]
        if not all(valid_path(path) for path in paths):
            raise PlanError('DIFF_FAILED', 'diff')
        changes.append({'status': status, 'paths': paths})
    return sorted(changes, key=lambda row: (row['paths'], row['status']))


def graph(metadata):
    """Normalize only snapshot roots; package IDs remain the graph keys."""
    try:
        root = Path(metadata['workspace_root'])
        if not root.is_absolute():
            raise ValueError()
        packages = {}
        members = set(metadata['workspace_members'])
        raw_packages = {p['id']: p for p in metadata['packages']}
        if not members <= raw_packages.keys() or not members:
            raise ValueError()

        def package_id(value):
            # Path IDs otherwise encode randomized snapshot paths in every plan.
            return value.replace(root.as_uri(), 'repo:').replace(str(root), '$REPO')

        reverse = {}
        builds = set()
        for raw_id, package in raw_packages.items():
            normalized = package_id(raw_id)
            reverse[normalized] = set()
            if raw_id not in members:
                continue
            manifest = Path(package['manifest_path']).relative_to(root).as_posix()
            if not valid_path(manifest) or not isinstance(package['name'], str) or not package['name']:
                raise ValueError()
            packages[normalized] = {'id': normalized, 'name': package['name'],
                                    'manifest_path': manifest}
            for target in package['targets']:
                if 'custom-build' in target.get('kind', []):
                    builds.add(Path(target['src_path']).relative_to(root).as_posix())
        seen = set()
        for node in metadata['resolve']['nodes']:
            if node['id'] not in raw_packages or node['id'] in seen:
                raise ValueError()
            seen.add(node['id'])
            for dependency in node['deps']:
                if dependency['pkg'] not in raw_packages:
                    raise ValueError()
                reverse[package_id(dependency['pkg'])].add(package_id(node['id']))
        if set(raw_packages) != seen or len(raw_packages) != len(metadata['packages']):
            raise ValueError()
        return packages, reverse, builds
    except (KeyError, TypeError, ValueError, AttributeError):
        raise PlanError('METADATA_FAILED', 'metadata') from None


def policy_inputs(policy, registry):
    try:
        valid = (policy['schema_version'] == 'nils-cli.merge-policy.v1'
                 and registry['schema_version'] == 'nils-cli.merge-extra-inputs.v1')
        for key in ('always_escalate', 'documentation'):
            valid = valid and isinstance(policy[key], list) and all(
                isinstance(p, str) and p for p in policy[key])
        if not valid or not isinstance(registry['rules'], list):
            raise ValueError()
        ids = set()
        for rule in registry['rules']:
            if not isinstance(rule['id'], str) or not rule['id'] or rule['id'] in ids:
                raise ValueError()
            ids.add(rule['id'])
            for key in ('paths', 'packages'):
                if (not isinstance(rule[key], list) or not rule[key]
                        or not all(isinstance(p, str) and p for p in rule[key])):
                    raise ValueError()
    except (KeyError, TypeError, ValueError):
        raise PlanError('POLICY_FAILED', 'policy') from None


def select(changes, base_metadata, candidate_metadata, policy, registry):
    policy_inputs(policy, registry)
    base_packages, base_reverse, base_builds = graph(base_metadata)
    packages, candidate_reverse, candidate_builds = graph(candidate_metadata)
    union = {**base_packages, **packages}
    reverse = {}
    for source in (base_reverse, candidate_reverse):
        for package_id, dependents in source.items():
            reverse.setdefault(package_id, set()).update(dependents)
    roots = sorted(((PurePosixPath(p['manifest_path']).parent.as_posix(), package_id)
                    for package_id, p in union.items()), key=lambda row: (-len(row[0]), row))
    selected = set()
    reasons = set()
    full = False

    def reason(code, path='', package_id=''):
        reasons.add((code, path, package_id))

    def escalate(code, path, package_id=''):
        nonlocal full
        full = True
        reason(code, path, package_id)

    def matches(path, patterns):
        return any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)

    for change in changes:
        for path in change['paths']:
            if not valid_path(path):
                raise PlanError('DIFF_FAILED', 'diff')
            if path in base_builds | candidate_builds or matches(path, policy['always_escalate']):
                escalate('ALWAYS_ESCALATE', path)
                continue
            owners = {pid for root, pid in roots if root == '.' or path == root or path.startswith(root + '/')}
            # Nested workspace members have the closest owner, on each tree.
            for tree_packages in (base_packages, packages):
                tree_roots = [(root, pid) for root, pid in roots if pid in tree_packages and pid in owners]
                if tree_roots:
                    closest = len(tree_roots[0][0])
                    owners -= {pid for root, pid in tree_roots if len(root) < closest}
            for pid in owners:
                if pid not in packages:
                    escalate('DELETED_PACKAGE', path, pid)
                else:
                    selected.add(pid)
                    reason('CHANGED_PACKAGE', path, pid)
            registered = False
            for rule in registry['rules']:
                if not matches(path, rule['paths']):
                    continue
                registered = True
                for name in rule['packages']:
                    targets = {pid for pid, p in union.items() if p['name'] == name}
                    if not targets or any(pid not in packages for pid in targets):
                        escalate('UNMAPPABLE_EXTRA_INPUT', path)
                    else:
                        selected.update(targets)
                        for pid in targets:
                            reason('EXTRA_INPUT:' + rule['id'], path, pid)
            if not owners and not registered:
                if matches(path, policy['documentation']):
                    reason('DOCUMENTATION', path)
                else:
                    escalate('UNKNOWN_PATH', path)

    pending = sorted(selected)
    reached = set(selected)
    while pending:
        source = pending.pop()
        for consumer in sorted(reverse.get(source, ())):
            if consumer in union:
                if consumer not in packages:
                    escalate('DELETED_PACKAGE', '', consumer)
                else:
                    selected.add(consumer)
                    reason('REVERSE_DEPENDENCY', '', consumer)
            if consumer not in reached:
                reached.add(consumer)
                pending.append(consumer)
    if full:
        selected = set(packages)
    return {'scope': 'FULL' if full else ('SCOPED' if changes else 'NONE'),
            'selected_packages': [packages[pid] for pid in sorted(selected)],
            'reasons': [{'code': code, 'path': path, 'package_id': pid}
                        for code, path, pid in sorted(reasons)]}


def git(repo, *args):
    try:
        return subprocess.run(['git', '--no-replace-objects', '-C', str(repo), *args], check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout
    except (OSError, subprocess.CalledProcessError):
        raise PlanError('GIT_FAILED', 'git') from None


def snapshot(repo, commit, destination):
    """Materialize blobs, preserving modes/symlinks, without export-ignore attrs."""
    entries = git(repo, 'ls-tree', '-r', '-z', commit).split(b'\0')
    for entry in entries:
        if not entry:
            continue
        info, raw_path = entry.split(b'\t', 1)
        mode, kind, oid = info.decode('ascii').split()
        path = raw_path.decode('utf-8', errors='surrogateescape')
        if kind != 'blob' or not valid_path(path):
            raise PlanError('SNAPSHOT_FAILED', 'snapshot')
        target = destination / path
        target.parent.mkdir(parents=True, exist_ok=True)
        content = git(repo, 'cat-file', 'blob', oid)
        if mode == '120000':
            target.symlink_to(content.decode('utf-8', errors='surrogateescape'))
        elif mode in ('100644', '100755'):
            target.write_bytes(content)
            target.chmod(0o755 if mode == '100755' else 0o644)
        else:
            raise PlanError('SNAPSHOT_FAILED', 'snapshot')


def metadata(snapshot_root, stage):
    try:
        result = subprocess.run(['cargo', 'metadata', '--offline', '--locked', '--all-features',
                                 '--format-version', '1'], cwd=snapshot_root, check=True,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env=dict(os.environ, RUSTUP_AUTO_INSTALL='0'))
        value = json.loads(result.stdout)
        if Path(value['workspace_root']).resolve() != snapshot_root.resolve():
            raise ValueError()
        packages, reverse, builds = graph(value)
        # The normalized graph, rather than random materialization paths, is the
        # metadata evidence. All resolved edges and custom build inputs are bound.
        normalized = {'packages': packages, 'reverse_dependencies':
                      {pid: sorted(consumers) for pid, consumers in reverse.items()},
                      'build_scripts': sorted(builds)}
        return value, digest(canonical(normalized))
    except (OSError, subprocess.CalledProcessError, KeyError, TypeError, ValueError, PlanError):
        raise PlanError('METADATA_FAILED', stage) from None


def generate(repo, inputs, selector_digest):
    plan = {'schema_version': VERSION, 'status': 'PLAN_ERROR', 'action': 'HOLD',
            'scope': None, 'inputs': inputs, 'digests': {'selector': selector_digest},
            'changes': [], 'selected_packages': [], 'reasons': [], 'error': None}
    try:
        policy_raw = git(repo, 'show', inputs['base'] + ':' + POLICY_PATH)
        registry_raw = git(repo, 'show', inputs['base'] + ':' + REGISTRY_PATH)
        schema_raw = git(repo, 'show', inputs['base'] + ':' + SCHEMA_PATH)
        plan['digests'].update(policy=digest(policy_raw), registry=digest(registry_raw), schema=digest(schema_raw))
        try:
            policy = json.loads(policy_raw)
            registry = json.loads(registry_raw)
            schema = json.loads(schema_raw)
            if schema['properties']['schema_version']['const'] != VERSION:
                raise ValueError()
        except (ValueError, KeyError, TypeError):
            raise PlanError('POLICY_FAILED', 'policy') from None
        policy_inputs(policy, registry)
        raw_diff = git(repo, 'diff', '--no-ext-diff', '--no-textconv', '--name-status', '-z',
                       '--find-renames=50%', '-l0', inputs['base'], inputs['candidate'], '--')
        plan['digests']['diff'] = digest(raw_diff)
        plan['changes'] = parse_diff(raw_diff)
        with tempfile.TemporaryDirectory(prefix='merge-plan-') as directory:
            base_dir = Path(directory) / 'base'
            candidate_dir = Path(directory) / 'candidate'
            base_dir.mkdir()
            candidate_dir.mkdir()
            snapshot(repo, inputs['base'], base_dir)
            snapshot(repo, inputs['candidate'], candidate_dir)
            base_metadata, plan['digests']['base_metadata'] = metadata(base_dir, 'base_metadata')
            candidate_metadata, plan['digests']['candidate_metadata'] = metadata(candidate_dir, 'candidate_metadata')
            plan.update(select(plan['changes'], base_metadata, candidate_metadata, policy, registry))
        plan.update(status='READY', action='VALIDATE')
    except PlanError as error:
        plan.update(scope=None, selected_packages=[], reasons=[])
        plan['error'] = {'code': error.code, 'stage': error.stage, 'retryable': True,
                         'next_action': 'repair_plan_inputs', 'recovery': {}}
    return plan
