"""Trusted-base affected merge selector. This module never runs validation gates."""
import fnmatch
import hashlib
import json
import os
import posixpath
import re
import shutil
import unicodedata
from pathlib import Path, PurePosixPath
import subprocess
import tempfile

VERSION = 'nils-cli.merge-plan.v2'
POLICY_PATH = 'scripts/ci/merge-plan-policy.v1.json'
REGISTRY_PATH = 'scripts/ci/merge-plan-extra-inputs.v1.json'
SCHEMA_PATH = 'scripts/ci/merge-plan.schema.v2.json'


class PlanError(Exception):
    def __init__(self, code, stage, recovery=None):
        self.code = code
        self.stage = stage
        self.recovery = recovery or {}


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
        bins = {}
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
            bins[normalized] = sorted(t['name'] for t in package['targets']
                                      if 'bin' in t.get('kind', []))
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
        return packages, reverse, builds, bins
    except (KeyError, TypeError, ValueError, AttributeError):
        raise PlanError('METADATA_FAILED', 'metadata') from None


def policy_inputs(policy, registry):
    try:
        valid = (policy['schema_version'] == 'nils-cli.merge-policy.v1'
                 and registry['schema_version'] == 'nils-cli.merge-extra-inputs.v1')
        for key in ('always_escalate', 'documentation', 'audit_only', 'completion_assets'):
            valid = valid and isinstance(policy[key], list) and all(
                isinstance(p, str) and p for p in policy[key])
        valid = valid and all(p.count('{bin}') == 1 for p in policy['completion_assets'])
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
    base_packages, base_reverse, base_builds, _ = graph(base_metadata)
    packages, candidate_reverse, candidate_builds, candidate_bins = graph(candidate_metadata)
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

    def completion_binary(path):
        for template in policy['completion_assets']:
            prefix, suffix = template.split('{bin}')
            name = path[len(prefix):len(path) - len(suffix)]
            if (path.startswith(prefix) and path.endswith(suffix)
                    and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]*', name)):
                return name
        return None

    for change in changes:
        for path in change['paths']:
            if not valid_path(path):
                raise PlanError('DIFF_FAILED', 'diff')
            if path in base_builds | candidate_builds:
                escalate('ALWAYS_ESCALATE', path)
                continue
            # The scoped lane runs the audit that reads these baselines over the
            # whole workspace, so they select no package and do not escalate.
            if path in policy['audit_only']:
                reason('AUDIT_ONLY', path)
                continue
            # A generated completion asset follows the one package that owns its
            # binary. Any other completions path falls through to ALWAYS_ESCALATE.
            binary = completion_binary(path)
            if binary is not None:
                owners = [pid for pid in packages if binary in candidate_bins[pid]]
                if len(owners) == 1:
                    selected.add(owners[0])
                    reason('COMPLETION_OWNER', path, owners[0])
                else:
                    escalate('UNKNOWN_COMPLETION_OWNER', path)
                continue
            if matches(path, policy['always_escalate']):
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
    if not full and selected and selected == set(packages):
        escalate('ALL_WORKSPACE_PACKAGES', '')
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
    """Preflight portable names, then create blobs through no-follow dir fds."""
    entries = []
    aliases = {}
    links = []
    root_fd = None
    try:
        for entry in git(repo, 'ls-tree', '-r', '-z', commit).split(b'\0'):
            if not entry:
                continue
            info, raw_path = entry.split(b'\t', 1)
            mode, kind, oid = info.decode('ascii').split()
            path = raw_path.decode('utf-8', errors='surrogateescape')
            if kind != 'blob' or mode not in ('100644', '100755', '120000') or not valid_path(path):
                raise ValueError()
            parts = path.split('/')
            # Reject aliases even on case-sensitive hosts so one tree has one
            # decision on supported filesystems, including Unicode normalization.
            for count in range(1, len(parts) + 1):
                prefix = '/'.join(parts[:count])
                key = unicodedata.normalize('NFD', prefix).casefold()
                if key in aliases and aliases[key] != prefix:
                    raise ValueError()
                aliases[key] = prefix
            entries.append((mode, oid, path))

        root_fd = os.open(destination, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        for mode, oid, path in entries:
            content = git(repo, 'cat-file', 'blob', oid)
            parts = path.split('/')
            parent_fd = os.dup(root_fd)
            try:
                for part in parts[:-1]:
                    try:
                        os.mkdir(part, 0o755, dir_fd=parent_fd)
                    except FileExistsError:
                        pass
                    next_fd = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                      dir_fd=parent_fd)
                    os.close(parent_fd)
                    parent_fd = next_fd
                if mode == '120000':
                    target = content.decode('utf-8', errors='surrogateescape')
                    normalized = posixpath.normpath(posixpath.join(posixpath.dirname(path), target))
                    if (not target or target.startswith('/') or normalized == '..'
                            or normalized.startswith('../')):
                        raise ValueError()
                    os.symlink(target, parts[-1], dir_fd=parent_fd)
                    links.append(destination / path)
                else:
                    permissions = 0o755 if mode == '100755' else 0o644
                    fd = os.open(parts[-1], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                 permissions, dir_fd=parent_fd)
                    with os.fdopen(fd, 'wb') as output:
                        output.write(content)
                        os.fchmod(output.fileno(), permissions)
            finally:
                os.close(parent_fd)
        for link in links:
            if not link.resolve().is_relative_to(destination.resolve()):
                raise ValueError()
    except (OSError, ValueError, RuntimeError):
        raise PlanError('SNAPSHOT_FAILED', 'snapshot') from None
    finally:
        if root_fd is not None:
            os.close(root_fd)


def metadata_configuration(base_root, candidate_root, changes):
    # Changed Cargo/toolchain configuration cannot be evaluated safely with the
    # old trusted context while claiming an exact candidate metadata graph.
    for change in changes:
        for path in change['paths']:
            parts = PurePosixPath(path).parts
            if ('.cargo' in parts or parts[-1] in ('rust-toolchain', 'rust-toolchain.toml')):
                raise PlanError('METADATA_FAILED', 'candidate_configuration')
    # Config can include other files, select programs, or override dependencies
    # through paths relative to a snapshot. Until those inputs can be trusted
    # without changing the candidate graph, HOLD even on unchanged root config.
    for root in (base_root, candidate_root):
        if any((root / path).exists() for path in ('.cargo/config', '.cargo/config.toml')):
            raise PlanError('METADATA_FAILED', 'candidate_configuration')
    # Also bind toolchain bytes when its root file is an internal symlink.
    for path in ('rust-toolchain', 'rust-toolchain.toml'):
        base = base_root / path
        candidate = candidate_root / path
        try:
            before = base.read_bytes() if base.is_file() else None
            after = candidate.read_bytes() if candidate.is_file() else None
        except OSError:
            raise PlanError('METADATA_FAILED', 'candidate_configuration') from None
        if before != after:
            raise PlanError('METADATA_FAILED', 'candidate_configuration')


def metadata_stderr_tail(raw):
    """Only producer-owned Cargo explanations may enter a public artifact."""
    text = raw.decode('utf-8', errors='replace') if isinstance(raw, bytes) else (raw or '')
    text = re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]', '', text)
    explanations = (
        (r'no matching package named', 'no matching package found in offline registry cache'),
        (r'(?:HTTP request|download).*offline|offline.*(?:fetch|download)', 'dependency download blocked by offline mode'),
        (r'lock ?file.*needs.*updat|lock file.*needs.*updat', 'lockfile update required'),
        (r'toolchain.*(?:not installed|not found|unavailable)', 'requested Rust toolchain unavailable'),
        (r'(?:failed to|could not).*execute.*rustc', 'Rust compiler probe could not execute'),
        (r'failed to parse (?:manifest|lock)|invalid TOML', 'manifest or lockfile could not be parsed'),
        (r'failed to load source for a dependency|failed to get .*as a dependency', 'dependency source unavailable'),
    )
    lines = []
    for line in text.splitlines(keepends=True):
        explanation = next((message for pattern, message in explanations if re.search(pattern, line, re.I)), None)
        prefix = 'error:' if line.lstrip().startswith('error:') else 'diagnostic:'
        rendered = 'error: ' + explanation if explanation else prefix + ' <redacted diagnostic line>'
        lines.append(rendered + ('\n' if line.endswith(('\n', '\r')) else ''))
    # No source-derived prose survives, including unquoted paths with spaces.
    return ''.join(lines).encode('utf-8')[-2048:].decode('utf-8', errors='ignore')


def metadata(snapshot_root, stage, trusted_root):
    def failure(kind, **details):
        return PlanError('METADATA_FAILED', stage, dict(kind=kind, **details))

    try:
        cargo = shutil.which('cargo')
        rustc = shutil.which('rustc')
        for tool, executable in (('cargo', cargo), ('rustc', rustc)):
            if executable is None:
                raise failure('missing_tool', tool=tool)
        # Both manifests use B's cwd/config/toolchain. M's .cargo and programs
        # cannot choose compiler tooling; ambient executables are caller-trusted.
        environment = dict(os.environ, RUSTUP_AUTO_INSTALL='0',
                           RUSTC=str(Path(rustc).absolute()),
                           RUSTC_WRAPPER='', RUSTC_WORKSPACE_WRAPPER='',
                           CARGO_BUILD_RUSTC=str(Path(rustc).absolute()),
                           CARGO_BUILD_RUSTC_WRAPPER='', CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER='')
        command = [str(Path(cargo).absolute()), 'metadata', '--manifest-path',
                   str(snapshot_root / 'Cargo.toml'), '--locked',
                   '--all-features', '--format-version', '1']
        if os.environ.get('NILS_CLI_MERGE_PLAN_PREPARE_METADATA') == '1':
            # Restored build caches need not contain every locked dependency
            # used by either all-platform graph. Prepare each exact snapshot
            # without executing build scripts, using the same trusted tooling.
            subprocess.run(command, cwd=trusted_root, check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env=environment)
        # Selection always consumes a fresh locked, offline graph; preparation
        # output is never substituted for the metadata bound into the plan.
        result = subprocess.run(command[:4] + ['--offline'] + command[4:],
                                cwd=trusted_root, check=True, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, env=environment)
    except OSError as error:
        details = {'errno': error.errno} if isinstance(error.errno, int) else {}
        raise failure('spawn_error', **details) from None
    except subprocess.CalledProcessError as error:
        raise failure('cargo_exit', exit_code=error.returncode,
                      stderr_tail=metadata_stderr_tail(error.stderr)) from None
    try:
        value = json.loads(result.stdout)
    except (ValueError, TypeError):
        raise failure('json_invalid') from None
    try:
        if Path(value['workspace_root']).resolve() != snapshot_root.resolve():
            raise ValueError()
    except (OSError, KeyError, TypeError, ValueError):
        raise failure('workspace_root_mismatch') from None
    try:
        packages, reverse, builds, bins = graph(value)
    except PlanError:
        raise failure('graph_error') from None
    # The normalized graph, rather than random materialization paths, is the
    # metadata evidence. All resolved edges and custom build inputs are bound.
    normalized = {'packages': packages, 'reverse_dependencies':
                  {pid: sorted(consumers) for pid, consumers in reverse.items()},
                  'build_scripts': sorted(builds), 'binaries': bins}
    return value, digest(canonical(normalized))


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
            metadata_configuration(base_dir, candidate_dir, plan['changes'])
            base_metadata, plan['digests']['base_metadata'] = metadata(base_dir, 'base_metadata', base_dir)
            candidate_metadata, plan['digests']['candidate_metadata'] = metadata(candidate_dir, 'candidate_metadata', base_dir)
            plan.update(select(plan['changes'], base_metadata, candidate_metadata, policy, registry))
        plan.update(status='READY', action='VALIDATE')
    except PlanError as error:
        plan.update(scope=None, selected_packages=[], reasons=[])
        plan['error'] = {'code': error.code, 'stage': error.stage, 'retryable': True,
                         'next_action': 'repair_plan_inputs', 'recovery': error.recovery}
    return plan
