#!/usr/bin/env python3
"""Host resource admission for repository checks (POSIX, Python standard library)."""
import contextlib
import datetime
import fcntl
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import time
import uuid

GIB = 1024 ** 3
SYSTEMD_RUN = '/usr/bin/systemd-run'
SYSTEMCTL = '/usr/bin/systemctl'
PROC_SELF = Path('/proc/self')


def current_cgroup():
    """Resolve kernel membership against the current mount namespace, never PATH/env."""
    def read_proc(name):
        try:
            return (PROC_SELF / name).read_text()
        except OSError as error:
            raise RuntimeError(f'existing-cgroup cannot read /proc/self/{name}; '
                               'provide readable procfs and a read-only cgroup v2 mount') from error

    groups = [line[3:] for line in read_proc('cgroup').splitlines() if line.startswith('0::')]
    if len(groups) != 1:
        raise RuntimeError('existing-cgroup requires current cgroup v2 membership (0::); '
                           'cgroup v1 is unsupported')

    def kernel_path(value):
        value = re.sub(r'\\([0-7]{3})', lambda match: chr(int(match[1], 8)), value)
        if not value.startswith('/') or '..' in value.split('/'):
            raise RuntimeError('existing-cgroup cannot resolve an unsafe kernel cgroup path')
        return Path(value)

    group = kernel_path(groups[0])
    mounts = []
    for line in read_proc('mountinfo').splitlines():
        before, separator, after = line.partition(' - ')
        fields, filesystem = before.split(), after.split()
        if not separator or not filesystem or filesystem[0] != 'cgroup2' or len(fields) < 6:
            continue
        root, mount = kernel_path(fields[3]), kernel_path(fields[4])
        try:
            relative = group.relative_to(root)
        except ValueError:
            continue
        mounts.append((len(root.parts), mount / relative, fields[5].split(',')))
    if not mounts:
        raise RuntimeError('existing-cgroup cannot resolve current membership on a cgroup v2 mount; '
                           'mount the current cgroup subtree read-only in this namespace')
    _, directory, options = max(mounts, key=lambda entry: entry[0])
    if 'ro' not in options:
        raise RuntimeError('existing-cgroup requires a read-only cgroup v2 mount; '
                           'the caller must externally enforce an unchangeable hard cap')
    return directory


def existing_cgroup_limits(maximum, high):
    """Observe the actual local limits; provisioning and immutable containment are external."""
    directory = current_cgroup()
    limits = {}
    for name, ceiling in (('memory.max', maximum * GIB), ('memory.swap.max', 0),
                          ('memory.high', high * GIB)):
        try:
            value = (directory / name).read_text().strip()
        except FileNotFoundError as error:
            if name == 'memory.high':
                limits[name] = None
                continue
            raise RuntimeError(f'existing-cgroup cannot read required {name}; '
                               'expose the current cgroup v2 memory limits read-only') from error
        except OSError as error:
            raise RuntimeError(f'existing-cgroup cannot read {name}; '
                               'expose readable cgroup v2 memory limits') from error
        if not re.fullmatch(r'[0-9]+', value) or int(value) > ceiling:
            raise RuntimeError(f'existing-cgroup {name} must be a finite byte limit <= {ceiling}; '
                               'configure compliant kernel limits before starting the gate')
        limits[name] = int(value)
    return limits


def validate_systemd():
    for executable in (SYSTEMD_RUN, SYSTEMCTL):
        path = Path(executable)
        metadata = path.lstat()
        if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0 or
                metadata.st_mode & 0o022 or not os.access(path, os.X_OK)):
            raise RuntimeError('Linux gate requires trusted systemd user-scope tools')


def positive(name, default, maximum=86400):
    value = int(os.environ.get(name, default))
    if not 1 <= value <= maximum:
        raise ValueError(f'{name} must be between 1 and {maximum}')
    return value


def memory_bytes(field):
    for line in Path('/proc/meminfo').read_text().splitlines():
        if line.startswith(f'{field}:'):
            return int(line.split()[1]) * 1024
    raise RuntimeError(f'{field} is unavailable; cannot admit a gate safely')


def available_memory():
    return memory_bytes('MemAvailable')


def total_memory():
    return memory_bytes('MemTotal')


def runner_limit(cpus, memory):
    return max(1, min(2, cpus // 4, memory // (4 * GIB)))


def state_directory():
    root = Path(os.environ.get('XDG_STATE_HOME', Path.home() / '.local/state'))
    directory = Path(os.environ['NILS_CLI_RESOURCE_STATE_DIR']) if 'NILS_CLI_RESOURCE_STATE_DIR' in os.environ else root / 'nils-cli' / 'resources'
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    return directory


def lock_file(path):
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    return os.fdopen(fd, 'r+')


def try_lock(file):
    try:
        fcntl.flock(file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except BlockingIOError:
        return False


def write_owner(file, slice_name=None):
    file.seek(0)
    file.truncate()
    json.dump({'pid': os.getpid(), 'slice': slice_name, 'since': datetime.datetime.now(
        datetime.timezone.utc).isoformat(timespec='seconds')}, file)
    file.flush()


def slice_quiescent(unit):
    if not re.fullmatch(r'nilsgate[0-9a-f]{32}\.slice', unit):
        raise ValueError('invalid gate slice in slot metadata')
    output = subprocess.check_output([SYSTEMCTL, '--user', 'show', unit,
        '--property=LoadState', '--property=ActiveState', '--property=ControlGroup',
        '--property=Job'], text=True, timeout=2)
    properties = dict(line.split('=', 1) for line in output.splitlines() if '=' in line)
    if properties.get('Job') not in ('', '0'):
        return False
    if properties.get('LoadState') == 'not-found':
        return True
    if properties.get('ActiveState') not in ('inactive', 'failed'):
        return False
    group = properties.get('ControlGroup', '')
    if not group:
        return True
    if not group.startswith('/') or '..' in group.split('/'):
        raise ValueError('invalid gate cgroup identity')
    events = Path('/sys/fs/cgroup') / group.lstrip('/') / 'cgroup.events'
    try:
        return 'populated 0' in events.read_text().splitlines()
    except FileNotFoundError:
        return True


class GateSemaphore:
    """FIFO among live tickets; kernel locks, rather than PIDs, prove ownership."""
    def __init__(self, directory, slots, quiescent=slice_quiescent):
        self.directory = directory
        self.slots = slots
        self.slot = None
        self.quiescent = quiescent

    def holders(self):
        holders = []
        for index in range(self.slots):
            with lock_file(self.directory / f'gate-{index}.lock') as file:
                free = try_lock(file)
                try:
                    owner = json.load(file)
                    if free and (not owner.get('slice') or self.quiescent(owner['slice'])):
                        continue
                    holders.append(f'pid={int(owner["pid"])} since={owner["since"]}')
                except (ValueError, KeyError):
                    if free:
                        continue
                    holders.append('holder metadata pending')
        return ', '.join(holders) or 'none (waiting for memory or earlier queued caller)'

    def acquire(self, timeout, ready=lambda: True, slice_name=None):
        deadline = time.monotonic() + timeout
        # Publish only while scanners are fenced: an unlocked visible ticket
        # would otherwise look like a crashed waiter.
        with lock_file(self.directory / 'admission.lock') as admission:
            fcntl.flock(admission, fcntl.LOCK_EX)
            ticket_path = self.directory / f'queue-{time.monotonic_ns():020d}-{uuid.uuid4().hex}.lock'
            ticket = lock_file(ticket_path)
            fcntl.flock(ticket, fcntl.LOCK_EX)
        last_notice = 0
        try:
            while True:
                with lock_file(self.directory / 'admission.lock') as admission:
                    fcntl.flock(admission, fcntl.LOCK_EX)
                    first = None
                    for path in sorted(self.directory.glob('queue-*.lock')):
                        if path == ticket_path:
                            first = path
                            break
                        with lock_file(path) as queued:
                            if try_lock(queued):
                                path.unlink(missing_ok=True)  # crashed waiter
                            else:
                                first = path
                                break
                    if first == ticket_path and ready():
                        for index in range(self.slots):
                            slot = lock_file(self.directory / f'gate-{index}.lock')
                            if try_lock(slot):
                                previous = slot.read()
                                try:
                                    owner = json.loads(previous) if previous else {}
                                except ValueError:
                                    owner = {}  # interrupted write before any scope launch
                                if not isinstance(owner, dict):
                                    owner = {}
                                if owner.get('slice') and not self.quiescent(owner['slice']):
                                    slot.close()
                                    continue  # crashed supervisor; scope/services still draining
                                self.slot = slot
                                write_owner(slot, slice_name)
                                return self
                            slot.close()
                now = time.monotonic()
                if now >= deadline:
                    raise TimeoutError(f'gate queue timed out after {timeout}s; holders: {self.holders()}')
                if now - last_notice >= 5:
                    print(f'gate queued; holders: {self.holders()}', file=sys.stderr, flush=True)
                    last_notice = now
                time.sleep(min(.1, max(0, deadline - now)))
        finally:
            ticket_path.unlink(missing_ok=True)
            ticket.close()

    def close(self):
        if self.slot is not None:
            self.slot.close()
            self.slot = None


def scope_command(command, slice_name, high, maximum):
    return [SYSTEMD_RUN, '--user', '--scope', '--quiet',
            f'--slice={slice_name}', f'--property=MemoryHigh={high}G',
            f'--property=MemoryMax={maximum}G', '--property=MemorySwapMax=0',
            '--property=OOMPolicy=kill', '--', *command]


def slice_command(slice_name, high, maximum):
    # Services started through the user manager are siblings of the scope.
    # A cap on their shared slice accounts for both services and scope.
    return [SYSTEMCTL, '--user', 'set-property', '--runtime', slice_name,
            f'MemoryHigh={high}G', f'MemoryMax={maximum}G',
            'MemorySwapMax=0']


def stop_process_group(worker):
    try:
        os.killpg(worker.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        worker.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    finally:
        # The leader may have exited while a descendant ignores SIGTERM.
        try:
            os.killpg(worker.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        worker.wait()


def run_gate(command, linux, admission_mode='systemd'):
    maximum = positive('NILS_CLI_GATE_MEMORY_MAX_GIB', 16, 1048576)
    high = positive('NILS_CLI_GATE_MEMORY_HIGH_GIB', max(1, maximum * 3 // 4), maximum)
    existing = admission_mode == 'existing-cgroup'
    if existing:
        existing_cgroup_limits(maximum, high)
    minimum = positive('NILS_CLI_GATE_MIN_AVAILABLE_GIB', 20, 1048576)
    timeout = positive('NILS_CLI_GATE_TIMEOUT_SECONDS', 3600)
    slots = positive('NILS_CLI_GATE_SLOTS', 1, 64)
    threads = positive('NILS_CLI_RUNNER_MAX', runner_limit(os.cpu_count() or 1,
                       available_memory() if linux else 8 * GIB), 64)
    if linux and threads == 1 and os.environ.get('NILS_CLI_CONTAINED_RUNNER_ACTIVE') == '1':
        raise RuntimeError('a complete gate inside a contained runner requires NILS_CLI_RUNNER_MAX>=2')
    if linux and minimum * GIB > total_memory():
        raise RuntimeError('NILS_CLI_GATE_MIN_AVAILABLE_GIB exceeds host memory capacity; '
                           'lower the admission floor and memory caps for a smaller dedicated host')
    jobs = min(threads, positive('CARGO_BUILD_JOBS', 1, 64))
    # Respect more restrictive caller test settings, and bound both runners.
    nextest = positive('NEXTEST_TEST_THREADS', threads, 1048576)
    rust = positive('RUST_TEST_THREADS', threads, 1048576)
    os.environ.update(NILS_CLI_RESOURCE_STATE_DIR=str(state_directory()),
                      NILS_CLI_RUNNER_MAX=str(threads), CARGO_BUILD_JOBS=str(jobs),
                      NEXTEST_TEST_THREADS=str(min(threads, nextest)),
                      RUST_TEST_THREADS=str(min(threads, rust)))
    ready = (lambda: available_memory() >= minimum * GIB) if linux else (lambda: True)
    slice_name = f'nilsgate{uuid.uuid4().hex}.slice' if linux and not existing else None
    with contextlib.closing(GateSemaphore(state_directory(), slots).acquire(
            timeout, ready, slice_name)) as lease:
        print(f'gate admitted: slots={slots} runners={threads} build_jobs={jobs} '
              f'memory_high={high}GiB memory_max={maximum}GiB', file=sys.stderr, flush=True)
        os.environ['NILS_CLI_GATE_ACTIVE'] = '1'
        os.environ['NILS_CLI_GATE_SUPERVISOR_PID'] = str(os.getpid())
        worker = [sys.executable, str(Path(__file__).resolve()), '--inside', '--', *command]
        if not linux or existing:
            if existing:
                # Recheck after queueing; only this explicit mode emits a receipt.
                limits = existing_cgroup_limits(maximum, high)
                print('gate admission receipt: ' + json.dumps({
                    'schema_version': 'nils-cli.gate-admission.v1',
                    'admission_mode': admission_mode, 'limits': limits}), file=sys.stderr, flush=True)
                worker[3:3] = ['--admission-mode', admission_mode]
                os.environ.pop('NILS_CLI_GATE_SLICE', None)
            else:
                print('gate: memory cgroup cap is a no-op on macOS; admission and concurrency limits remain active',
                      file=sys.stderr)
            # Retain crash ownership in the foreground worker only. Its child
            # command closes unrelated FDs before background helpers can fork.
            foreground = subprocess.Popen(worker, start_new_session=True,
                                          pass_fds=(lease.slot.fileno(),))
            try:
                return foreground.wait()
            except BaseException:
                # Retain admission while stopping every process in this gate.
                stop_process_group(foreground)
                raise
        os.environ['NILS_CLI_GATE_SLICE'] = slice_name
        scoped = scope_command(worker, slice_name, high, maximum)
        try:
            return subprocess.call(scoped, pass_fds=(lease.slot.fileno(),))
        finally:
            # Keep admission through cleanup, including services outside the scope.
            subprocess.run([SYSTEMCTL, '--user', 'stop', slice_name], check=True, timeout=30)


def main(arguments):
    inside = arguments[0:1] == ['--inside']
    if inside:
        arguments = arguments[1:]
    admission_mode = 'systemd'
    if arguments[0:1] == ['--admission-mode']:
        if len(arguments) < 2 or arguments[1] not in ('systemd', 'existing-cgroup'):
            raise ValueError('--admission-mode must be systemd or existing-cgroup')
        admission_mode = arguments[1]
        arguments = arguments[2:]
    if arguments[0:1] != ['--'] or len(arguments) < 2:
        raise ValueError('usage: gate-resources.py [--inside] -- command [args...]')
    command = arguments[1:]
    existing = admission_mode == 'existing-cgroup'
    if existing and sys.platform != 'linux':
        raise RuntimeError('existing-cgroup admission is supported only on Linux')
    if inside:
        # A late scope launch after supervisor death must not start a workload.
        os.kill(int(os.environ['NILS_CLI_GATE_SUPERVISOR_PID']), 0)
        if sys.platform == 'linux':
            maximum = positive('NILS_CLI_GATE_MEMORY_MAX_GIB', 16, 1048576)
            high = positive('NILS_CLI_GATE_MEMORY_HIGH_GIB', max(1, maximum * 3 // 4), maximum)
            if existing:
                existing_cgroup_limits(maximum, high)
            else:
                subprocess.run(slice_command(os.environ['NILS_CLI_GATE_SLICE'], high, maximum),
                               check=True, timeout=10)
        return subprocess.call(command)
    linux = sys.platform == 'linux'
    if existing:
        return run_gate(command, linux, admission_mode)
    if linux:
        validate_systemd()
    return run_gate(command, linux)


if __name__ == '__main__':
    try:
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f'error: gate resource admission failed: {error}', file=sys.stderr)
        sys.exit(2)
