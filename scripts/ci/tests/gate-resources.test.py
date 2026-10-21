#!/usr/bin/env python3
"""Resource admission regressions; fake workloads, no compiler or systemd."""
import contextlib
import importlib.util
import io
import json
import multiprocessing
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import DEFAULT, patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('gate_resources', ROOT / 'scripts/ci/gate-resources.py')
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)


def hold_slot(directory, started):
    lease = resources.GateSemaphore(Path(directory), 1).acquire(5)
    started.set()
    time.sleep(30)
    lease.close()


def legacy_first_ticket(directory, ticket_path):
    """Frozen pre-priority admission scanner, as shipped before issue #2556."""
    with resources.lock_file(directory / 'admission.lock') as admission:
        resources.fcntl.flock(admission, resources.fcntl.LOCK_EX)
        first = None
        for path in sorted(directory.glob('queue-*.lock')):
            if path == ticket_path:
                first = path
                break
            with resources.lock_file(path) as queued:
                if resources.try_lock(queued):
                    path.unlink(missing_ok=True)
                else:
                    first = path
                    break
        return first


def wait_until(predicate, timeout=5):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(.02)
    return predicate()


def process_running(pid):
    result = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)],
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    return result.returncode == 0 and any(
        line.strip() and not line.strip().startswith('Z') for line in result.stdout.splitlines())


class GateUnits(unittest.TestCase):
    @contextlib.contextmanager
    def cancelled_wait(self, diagnostic=None):
        output = io.StringIO()
        with patch.object(resources.time, 'sleep', side_effect=InterruptedError('cancel wait')), \
             contextlib.redirect_stderr(output):
            with self.assertRaisesRegex(InterruptedError, 'cancel wait'):
                yield
        if diagnostic:
            self.assertRegex(output.getvalue(), diagnostic)

    def test_timeout_retains_original_ticket_and_fifo_position(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            earlier = root / 'queue-00000000000000000000-earlier.lock'
            later = root / 'queue-99999999999999999999-later.lock'
            with resources.lock_file(earlier) as predecessor, resources.lock_file(later) as successor:
                resources.try_lock(predecessor)
                resources.try_lock(successor)
                clock = [100.0]
                original = []
                notices = io.StringIO()
                def advance(seconds):
                    clock[0] += seconds
                    tickets = [path for path in root.glob('queue-*.lock')
                               if path not in (earlier, later)]
                    self.assertEqual(len(tickets), 1)
                    self.assertEqual(json.loads(tickets[0].read_text())['priority'], 'normal')
                    identity = (tickets[0].name, tickets[0].stat().st_ino)
                    if not original:
                        original.append(identity)
                    self.assertEqual(identity, original[0], 'waiter lost its original ticket')
                    if clock[0] >= 100.5:
                        predecessor.close()
                    self.assertLess(clock[0], 102, 'waiter did not retain its FIFO position')
                with patch.object(resources.time, 'monotonic', side_effect=lambda: clock[0]), \
                     patch.object(resources.time, 'sleep', side_effect=advance), \
                     contextlib.redirect_stderr(notices):
                    lease = resources.GateSemaphore(root, 1).acquire(.2)
                    lease.close()
                self.assertGreaterEqual(clock[0], 100.5)
                self.assertIn('still queued', notices.getvalue())
                self.assertFalse([path for path in root.glob('queue-*.lock') if path != later])

    def test_urgent_ticket_passes_two_normal_waiters_and_is_auditable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with contextlib.ExitStack() as stack:
                for index in range(2):
                    ticket = stack.enter_context(resources.lock_file(
                        root / f'queue-{index:020d}-normal.lock'))
                    resources.try_lock(ticket)  # legacy metadata-free normal tickets
                env = {'XDG_STATE_HOME': directory, 'NILS_CLI_GATE_PRIORITY': 'urgent',
                       'NILS_CLI_GATE_TIMEOUT_SECONDS': '1'}
                def launch(*args, **kwargs):
                    semaphore = resources.GateSemaphore(resources.state_directory(), 1)
                    self.assertIn('priority=urgent', semaphore.holders())
                    return DEFAULT
                with patch.dict(os.environ, env, clear=True), \
                     patch.object(resources, 'state_directory', return_value=root), \
                     patch.object(resources.time, 'sleep',
                                  side_effect=AssertionError('urgent blocked by normal waiters')), \
                     patch.object(resources.subprocess, 'Popen', side_effect=launch) as process:
                    process.return_value.wait.return_value = 0
                    self.assertEqual(resources.run_gate(['true'], False), 0)

    def test_queued_priority_age_fifo_and_memory_protection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            earlier = root / 'queue-0-00000000000000000000-urgent.lock'
            with resources.lock_file(earlier) as predecessor:
                resources.try_lock(predecessor)
                resources.write_owner(predecessor)
                predecessor.seek(0)
                owner = json.load(predecessor)
                owner['priority'] = 'urgent'
                predecessor.seek(0)
                predecessor.truncate()
                json.dump(owner, predecessor)
                predecessor.flush()
                def inspect_wait(_):
                    semaphore = resources.GateSemaphore(root, 1)
                    output = semaphore.holders()
                    self.assertEqual(output.count('priority=urgent'), 2)
                    self.assertEqual(output.count('age='), 2)
                    tickets = sorted(root.glob('queue-*.lock'))
                    self.assertEqual(json.loads(tickets[-1].read_text())['priority'], 'urgent')
                    raise InterruptedError('cancel queued invocation')
                with patch.object(resources.time, 'sleep', side_effect=inspect_wait):
                    with self.assertRaisesRegex(InterruptedError, 'cancel queued invocation'):
                        resources.GateSemaphore(root, 1).acquire(.1, priority='urgent')
                predecessor.close()
                earlier.unlink()
            # Urgent admission still requires the memory predicate to pass.
            with patch.object(resources.time, 'sleep', side_effect=InterruptedError('memory blocked')):
                with self.assertRaisesRegex(InterruptedError, 'memory blocked'):
                    resources.GateSemaphore(root, 1).acquire(.1, lambda: False, priority='urgent')
            self.assertFalse(list(root.glob('queue-*.lock')))

    def test_urgent_waiter_does_not_preempt_a_running_holder(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            holder = resources.GateSemaphore(root, 1).acquire(.1)
            before = (root / 'gate-0.lock').read_text()
            try:
                with patch.object(resources.time, 'sleep', side_effect=InterruptedError('still held')):
                    with self.assertRaisesRegex(InterruptedError, 'still held'):
                        resources.GateSemaphore(root, 1).acquire(.1, priority='urgent')
                self.assertEqual((root / 'gate-0.lock').read_text(), before)
            finally:
                holder.close()

    def test_previous_normal_consumer_defers_to_live_urgent_after_holder_exits(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as stack:
            root = Path(directory)
            normal = []
            for index in range(2):
                path = root / f'queue-{index:020d}-legacy.lock'
                ticket = stack.enter_context(resources.lock_file(path))
                resources.try_lock(ticket)
                normal.append(path)
            # Obtain the holder independently of the queued normal callers.
            holder = resources.lock_file(root / 'gate-0.lock')
            resources.try_lock(holder)
            resources.write_owner(holder)
            old_slot = []
            def previous_consumer_runs_first(_):
                holder.close()
                # Schedule the frozen previous decision immediately after the
                # holder exits, before the new urgent consumer gets another turn.
                if legacy_first_ticket(root, normal[0]) == normal[0]:
                    slot = resources.lock_file(root / 'gate-0.lock')
                    old_slot.append(slot)
                    self.assertTrue(resources.try_lock(slot))
                    resources.write_owner(slot)
                self.assertFalse(old_slot, 'previous normal consumer admitted ahead of live urgent')
            try:
                with patch.object(resources.time, 'sleep', side_effect=previous_consumer_runs_first):
                    lease = resources.GateSemaphore(root, 1).acquire(.1, priority='urgent')
                    lease.close()
                self.assertEqual(legacy_first_ticket(root, normal[0]), normal[0])
            finally:
                holder.close()
                for slot in old_slot:
                    slot.close()

    def test_previous_scanner_preserves_urgent_and_legacy_fifo_and_queue_age(self):
        with tempfile.TemporaryDirectory() as directory, contextlib.ExitStack() as stack:
            root = Path(directory)
            normal = []
            for index in range(2):
                path = root / f'queue-{index:020d}-legacy.lock'
                ticket = stack.enter_context(resources.lock_file(path))
                resources.try_lock(ticket)
                normal.append(path)
            clock = [2_000_000_000]
            def monotonic_ns():
                clock[0] += 100_000
                return clock[0]
            def enqueue_second(_):
                first = next(path for path in root.glob('queue-*.lock') if path not in normal)
                def inspect_order(_):
                    second = next(path for path in root.glob('queue-*.lock')
                                  if path not in [*normal, first])
                    self.assertEqual(sorted(root.glob('queue-*.lock')), [first, second, *normal])
                    self.assertEqual(legacy_first_ticket(root, normal[0]), first)
                    diagnostics = resources.GateSemaphore(root, 1).holders()
                    urgent = [item for item in diagnostics.split('; ') if 'priority=urgent' in item]
                    self.assertEqual(len(urgent), 2)
                    for item in urgent:
                        self.assertIn('age=0s', item)
                    raise InterruptedError('cancel FIFO fixture')
                with patch.object(resources.time, 'sleep', side_effect=inspect_order):
                    with self.assertRaisesRegex(InterruptedError, 'cancel FIFO fixture'):
                        resources.GateSemaphore(root, 1).acquire(.1, lambda: False, priority='urgent')
                raise InterruptedError('cancel FIFO fixture')
            with patch.object(resources.time, 'monotonic_ns', side_effect=monotonic_ns), \
                 patch.object(resources.time, 'sleep', side_effect=enqueue_second):
                with self.assertRaisesRegex(InterruptedError, 'cancel FIFO fixture'):
                    resources.GateSemaphore(root, 1).acquire(.1, lambda: False, priority='urgent')
            self.assertEqual(list(sorted(root.glob('queue-*.lock'))), normal)
            self.assertEqual(legacy_first_ticket(root, normal[0]), normal[0])
            stack.close()
            self.assertIsNone(legacy_first_ticket(root, root / 'absent-ticket'))

    def test_invalid_priority_is_rejected_before_queueing(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, {'XDG_STATE_HOME': directory,
                                         'NILS_CLI_GATE_PRIORITY': 'unexpected'}, clear=True), \
                 patch.object(resources, 'GateSemaphore') as semaphore, \
                 patch.object(resources.subprocess, 'Popen'):
                with self.assertRaisesRegex(ValueError, 'NILS_CLI_GATE_PRIORITY'):
                    resources.run_gate(['true'], False)
                semaphore.assert_not_called()

    def test_impossible_memory_floor_fails_before_queueing(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.dict(os.environ, {'XDG_STATE_HOME': directory}, clear=True), \
                 patch.object(resources, 'available_memory', return_value=32 * resources.GIB), \
                 patch.object(resources, 'total_memory', return_value=8 * resources.GIB, create=True), \
                 patch.object(resources, 'GateSemaphore') as semaphore, \
                 patch.object(resources.subprocess, 'call', return_value=0), \
                 patch.object(resources.subprocess, 'run'):
                with self.assertRaisesRegex(RuntimeError, 'NILS_CLI_GATE_MIN_AVAILABLE_GIB.*capacity'):
                    resources.run_gate(['true'], True)
                semaphore.assert_not_called()
            self.assertFalse(list(Path(directory).rglob('queue-*.lock')))

    def test_mac_sigterm_stops_workload_and_descendant_before_readmission(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            helper = root / 'gate-resources.py'
            helper.write_text('import sys\nsys.platform = "darwin"\n' +
                              (ROOT / 'scripts/ci/gate-resources.py').read_text())
            parent_pid, descendant_pid = root / 'parent.pid', root / 'descendant.pid'
            descendant = ('import os, signal, sys, time; from pathlib import Path; '
                          'signal.signal(signal.SIGTERM, signal.SIG_IGN); '
                          'Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(30)')
            workload = ('import os, subprocess, sys, time; from pathlib import Path; '
                        'Path(sys.argv[1]).write_text(str(os.getpid())); '
                        'subprocess.Popen([sys.executable, "-c", sys.argv[3], sys.argv[2]]); '
                        'time.sleep(30)')
            state = root / 'state'
            env = dict(os.environ, NILS_CLI_RESOURCE_STATE_DIR=str(state), NILS_CLI_GATE_SLOTS='1')
            supervisor = subprocess.Popen(
                [sys.executable, str(helper), '--', sys.executable, '-c', workload,
                 str(parent_pid), str(descendant_pid), descendant], env=env,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                self.assertTrue(wait_until(lambda: parent_pid.exists() and descendant_pid.exists()))
                pids = [int(path.read_text()) for path in (parent_pid, descendant_pid)]
                supervisor.terminate()
                self.assertEqual(supervisor.wait(timeout=8), 143)
                self.assertFalse(any(map(process_running, pids)),
                                 'workload survives supervisor termination')
                lease = resources.GateSemaphore(state, 1).acquire(.5)
                self.assertFalse(any(map(process_running, pids)))
                lease.close()
            finally:
                for path in (parent_pid, descendant_pid):
                    if path.exists():
                        try:
                            os.kill(int(path.read_text()), signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                if supervisor.poll() is None:
                    supervisor.kill()
                supervisor.wait(timeout=5)

    def test_nested_single_slot_gate_fails_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            env = {'XDG_STATE_HOME': directory, 'NILS_CLI_RUNNER_MAX': '1',
                   'NILS_CLI_CONTAINED_RUNNER_ACTIVE': '1'}
            with patch.dict(os.environ, env, clear=True), \
                 patch.object(resources, 'available_memory', return_value=32 * resources.GIB), \
                 patch.object(resources.subprocess, 'call', return_value=0) as launch, \
                 patch.object(resources.subprocess, 'run'):
                with self.assertRaisesRegex(RuntimeError, 'NILS_CLI_RUNNER_MAX.*2'):
                    resources.run_gate(['true'], True)
                launch.assert_not_called()

    def test_mac_background_helper_does_not_retain_gate_slot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            helper = root / 'gate-resources.py'
            helper.write_text('import sys\nsys.platform = "darwin"\n' +
                              (ROOT / 'scripts/ci/gate-resources.py').read_text())
            state = root / 'state'
            pid_file = root / 'background.pid'
            foreground = (
                'import subprocess, sys; from pathlib import Path; '
                'child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], '
                'close_fds=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); '
                'Path(sys.argv[1]).write_text(str(child.pid))')
            env = dict(os.environ, NILS_CLI_RESOURCE_STATE_DIR=str(state))
            try:
                result = subprocess.run([sys.executable, str(helper), '--', sys.executable,
                                         '-c', foreground, str(pid_file)], env=env,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                                        timeout=5)
                self.assertEqual(result.returncode, 0, result.stderr.decode())
                os.kill(int(pid_file.read_text()), 0)  # helper remains alive
                lease = resources.GateSemaphore(state, 1).acquire(.2)
                lease.close()
            finally:
                if pid_file.exists():
                    try:
                        os.kill(int(pid_file.read_text()), signal.SIGTERM)
                    except ProcessLookupError:
                        pass

    def test_crashed_holder_and_stale_ticket_are_recovered(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            started = multiprocessing.Event()
            holder = multiprocessing.Process(target=hold_slot, args=(directory, started))
            holder.start()
            try:
                self.assertTrue(started.wait(5))
                with self.cancelled_wait('holders: pid=.*since='):
                    resources.GateSemaphore(root, 1).acquire(.1)
                holder.kill()
                holder.join(5)
                (root / 'queue-00000000000000000000-stale.lock').touch()
                lease = resources.GateSemaphore(root, 1).acquire(.5)
                lease.close()
                self.assertFalse(list(root.glob('queue-*')))
            finally:
                if holder.is_alive():
                    holder.kill()
                holder.join(5)

    def test_fifo_waiters_do_not_pass_an_earlier_live_ticket(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ticket = resources.lock_file(root / 'queue-00000000000000000000-live.lock')
            resources.try_lock(ticket)
            try:
                with self.cancelled_wait('gate queued'):
                    resources.GateSemaphore(root, 2).acquire(.1)
            finally:
                ticket.close()
            lease = resources.GateSemaphore(root, 2).acquire(.5)
            lease.close()

    def test_insufficient_memory_never_admits_work(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.cancelled_wait('waiting for memory'):
                resources.GateSemaphore(Path(directory), 1).acquire(.1, lambda: False)

    def test_scope_and_shared_slice_properties(self):
        unit = 'nilsgate' + 'a' * 32 + '.slice'
        command = resources.scope_command(['bash', 'checks.sh'], unit, 12, 16)
        self.assertEqual(command, [
            '/usr/bin/systemd-run', '--user', '--scope', '--quiet', f'--slice={unit}',
            '--property=MemoryHigh=12G', '--property=MemoryMax=16G',
            '--property=MemorySwapMax=0', '--property=OOMPolicy=kill',
            '--', 'bash', 'checks.sh'])
        self.assertEqual(resources.slice_command(unit, 12, 16), [
            '/usr/bin/systemctl', '--user', 'set-property', '--runtime', unit,
            'MemoryHigh=12G', 'MemoryMax=16G', 'MemorySwapMax=0'])

    def test_mac_fallback_propagates_bounded_threads_and_jobs(self):
        with tempfile.TemporaryDirectory() as directory:
            env = {'XDG_STATE_HOME': directory, 'NILS_CLI_RUNNER_MAX': '2',
                   'NEXTEST_TEST_THREADS': '32', 'RUST_TEST_THREADS': '1', 'CARGO_BUILD_JOBS': '64'}
            with patch.dict(os.environ, env, clear=True), patch.object(resources.subprocess, 'Popen') as launch:
                launch.return_value.wait.return_value = 0
                self.assertEqual(resources.run_gate(['true'], False), 0)
                self.assertEqual(os.environ['NEXTEST_TEST_THREADS'], '2')
                self.assertEqual(os.environ['RUST_TEST_THREADS'], '1')
                self.assertEqual(os.environ['CARGO_BUILD_JOBS'], '2')
                self.assertTrue(launch.call_args.kwargs['pass_fds'])
                self.assertTrue(launch.call_args.kwargs['start_new_session'])

    def test_ticket_publication_is_serialized_with_queue_scans(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = resources.lock_file
            def inspect_publication(path):
                if path.name.startswith('queue-'):
                    with original(root / 'admission.lock') as scanner:
                        self.assertFalse(resources.try_lock(scanner),
                                         'a scanner can see an unlocked live ticket')
                return original(path)
            with patch.object(resources, 'lock_file', side_effect=inspect_publication):
                lease = resources.GateSemaphore(root, 1).acquire(.5)
                lease.close()

    def test_interrupted_owner_metadata_write_is_recovered(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'gate-0.lock').write_text('{"pid":')
            lease = resources.GateSemaphore(root, 1).acquire(.5)
            lease.close()
            self.assertIn('since', (root / 'gate-0.lock').read_text())

    def test_crashed_gate_waits_for_its_slice_to_drain(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            lease = resources.GateSemaphore(root, 1).acquire(.5, slice_name='nilsgate' + 'a' * 32 + '.slice')
            lease.close()  # supervisor gone, slice still running
            with self.cancelled_wait('holders: pid='):
                resources.GateSemaphore(root, 1, quiescent=lambda _: False).acquire(.1)
            lease = resources.GateSemaphore(root, 1, quiescent=lambda _: True).acquire(.5)
            lease.close()

    def test_gate_holds_admission_until_slice_cleanup_returns(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = {'XDG_STATE_HOME': directory, 'NILS_CLI_RUNNER_MAX': '2'}
            def cleanup(*args, **kwargs):
                with self.cancelled_wait():
                    resources.GateSemaphore(resources.state_directory(), 1).acquire(.1)
            with patch.dict(os.environ, env, clear=True), \
                 patch.object(resources, 'available_memory', return_value=32 * resources.GIB), \
                 patch.object(resources, 'total_memory', return_value=32 * resources.GIB), \
                 patch.object(resources.subprocess, 'call', return_value=0), \
                 patch.object(resources.subprocess, 'run', side_effect=cleanup):
                self.assertEqual(resources.run_gate(['true'], True), 0)

    def test_path_shadows_cannot_select_the_noop_or_replace_systemd(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            marker = root / 'shadow-called'
            for name in ('uname', 'systemd-run', 'systemctl'):
                executable = root / name
                executable.write_text(f'#!/bin/sh\ntouch "{marker}"\necho Darwin\n')
                executable.chmod(0o755)
            with patch.dict(os.environ, {'PATH': directory}), \
                 patch.object(resources.sys, 'platform', 'linux'), \
                 patch.object(resources, 'validate_systemd') as validate, \
                 patch.object(resources, 'run_gate', return_value=0) as gate:
                self.assertEqual(resources.main(['--', 'true']), 0)
                validate.assert_called_once()
                gate.assert_called_once_with(['true'], True)
            self.assertFalse(marker.exists())
            self.assertEqual(resources.scope_command(['true'], 'unit', 12, 16)[0],
                             '/usr/bin/systemd-run')
            self.assertEqual(resources.slice_command('unit', 12, 16)[0], '/usr/bin/systemctl')

    def test_default_runner_limit_has_floor_and_ceiling(self):
        self.assertEqual(resources.runner_limit(1, 0), 1)
        self.assertEqual(resources.runner_limit(64, 64 * resources.GIB), 2)
        self.assertEqual(resources.runner_limit(8, 4 * resources.GIB), 1)


class GateIntegration(unittest.TestCase):
    def make_repository(self, root):
        ci = root / 'scripts/ci'
        ci.mkdir(parents=True)
        (ci / 'lib').mkdir()
        for name in ('nils-cli-checks-entrypoint.sh', 'nils-cli-local-fast.sh', 'test-env.sh'):
            shutil.copy(ROOT / 'scripts/ci' / name, ci / name)
        for name in ('doc_classify.py', 'sibling_bins.py', 'cargo-target-dir.sh', 'target_guard.py'):
            shutil.copy(ROOT / 'scripts/ci/lib' / name, ci / 'lib' / name)
        verify = root / '.agents/skills/project-verify-required-checks/scripts'
        verify.mkdir(parents=True)
        (verify / 'project-verify-required-checks.sh').write_text('touch "$DOCS_CALLED"\n')
        (ci / 'devlog-check.py').write_text('# Devlog policy is tested separately.\n')
        (ci / 'gate-resources.py').write_text(
            'import os, sys; from pathlib import Path; '
            'Path(os.environ["GATE_CALLED"]).write_text("called"); sys.exit(71)\n')
        subprocess.run(['git', 'init', '-q', str(root)], check=True)
        return dict(os.environ, NILS_CLI_GATE_ACTIVE='0',
                    GATE_CALLED=str(root / 'gate-called'), DOCS_CALLED=str(root / 'docs-called'))

    def test_local_fast_noncode_modes_and_usage_error_bypass_admission(self):
        cases = [(['--changed-file', 'README.md'], 0, 'docs-only'),
                 (['--changed-file', ''], 0, 'none'),
                 (['--with-coverage'], 2, None)]
        for arguments, expected, mode in cases:
            with self.subTest(arguments=arguments), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                env = self.make_repository(root)
                result = subprocess.run(
                    ['bash', 'scripts/ci/nils-cli-checks-entrypoint.sh', '--local-fast', *arguments],
                    cwd=root, env=env, capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertFalse((root / 'gate-called').exists(), 'noncode mode tried admission')
                if mode:
                    self.assertIn(f'LOCAL_FAST_MODE={mode}', result.stdout)
                if mode == 'docs-only':
                    self.assertTrue((root / 'docs-called').exists())

    def test_direct_code_gate_plans_once_without_temporary_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = self.make_repository(root)
            temporary = root / 'temporary'
            temporary.mkdir()
            binaries = root / 'bin'
            binaries.mkdir()
            cargo = binaries / 'cargo'
            cargo.write_text(
                '#!/usr/bin/env python3\nimport json, os; from pathlib import Path\n'
                'with Path(os.environ["METADATA_CALLS"]).open("a") as log: log.write("metadata\\n")\n'
                'packages = [{"id": name, "name": name, "manifest_path": '
                'str(Path.cwd()/f"crates/{name}/Cargo.toml"), "targets":[]} '
                'for name in ("example", "consumer")]\n'
                'print(json.dumps({"packages": packages, "workspace_members": ["example", "consumer"], '
                '"resolve": {"nodes": [{"id": "example", "deps": []}, '
                '{"id": "consumer", "deps": [{"pkg": "example"}]}]}}))\n')
            cargo.chmod(0o755)
            (root / 'scripts/ci/gate-resources.py').write_text(
                'import os, subprocess, sys; from pathlib import Path\n'
                'Path(os.environ["GATE_CALLED"]).write_text("called")\n'
                'env = dict(os.environ, NILS_CLI_GATE_ACTIVE="1")\n'
                'sys.exit(subprocess.call(sys.argv[2:] + ["--plan-only"], env=env))\n')
            env.update(PATH=str(binaries) + os.pathsep + env['PATH'], TMPDIR=str(temporary),
                       METADATA_CALLS=str(root / 'metadata-calls'))
            result = subprocess.run(
                ['bash', 'scripts/ci/nils-cli-local-fast.sh', '--changed-file', 'crates/example/src/lib.rs'],
                cwd=root, env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((root / 'gate-called').exists())
            self.assertEqual((root / 'metadata-calls').read_text().splitlines(), ['metadata'])
            self.assertEqual(result.stdout.count('LOCAL_FAST_MODE=packages'), 1)
            self.assertIn('LOCAL_FAST_PACKAGE=consumer', result.stdout)
            self.assertIn('LOCAL_FAST_PACKAGE_REASON=consumer: reverse dependency of example', result.stdout)
            self.assertFalse(list(temporary.glob('nils-cli-local-fast.*')))

    def test_second_gate_waits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'scripts/ci').mkdir(parents=True)
            (root / '.agents/skills/project-verify-required-checks/scripts').mkdir(parents=True)
            for name in ('nils-cli-checks-entrypoint.sh', 'test-env.sh', 'gate-resources.py'):
                source = ROOT / 'scripts/ci' / name
                if source.exists():
                    shutil.copy(source, root / 'scripts/ci' / name)
            (root / '.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh').write_text(
                'touch "$STARTED/$LABEL"\n'
                'if [[ "$LABEL" == "first" ]]; then\n'
                '  while [[ ! -f "$STARTED/release" ]]; do sleep .02; done\n'
                'fi\n')
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            # Exercise real admission with the macOS no-op memory path. This
            # test-only copied helper selects that runtime; production detects
            # sys.platform and never consults a PATH-provided uname command.
            helper = root / 'scripts/ci/gate-resources.py'
            helper.write_text('import sys\nsys.platform = "darwin"\n' + helper.read_text())
            env = dict(os.environ, XDG_STATE_HOME=str(root / 'state'),
                       STARTED=str(root),
                       NILS_CLI_GATE_SLOTS='1',
                       NILS_CLI_GATE_TIMEOUT_SECONDS='5', NILS_CLI_GATE_ACTIVE='0',
                       NILS_CLI_RESOURCE_STATE_DIR=str(root / 'state/resources'))
            command = ['bash', 'scripts/ci/nils-cli-checks-entrypoint.sh', '--fixture',
                       'path --plan-only marker.rs']
            with subprocess.Popen(command, cwd=root, env=dict(env, LABEL='first'),
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE) as first:
                try:
                    self.assertTrue(wait_until(lambda: (root / 'first').exists()),
                                    'first gate never started')
                    with subprocess.Popen(command, cwd=root, env=dict(env, LABEL='second'),
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE) as second:
                        try:
                            tickets = lambda: list((root / 'state/resources').glob('queue-*.lock'))
                            self.assertTrue(wait_until(lambda: tickets() or (root / 'second').exists()))
                            self.assertFalse((root / 'second').exists(),
                                             'second gate started while first held the host slot')
                            self.assertTrue(tickets(), 'second caller never reached the queue')
                            (root / 'release').touch()
                            self.assertEqual(first.wait(timeout=5), 0)
                            self.assertEqual(second.wait(timeout=5), 0)
                            self.assertTrue((root / 'second').exists())
                        finally:
                            (root / 'release').touch()
                            if second.poll() is None:
                                second.terminate()
                finally:
                    (root / 'release').touch()
                    if first.poll() is None:
                        first.terminate()


if __name__ == '__main__':
    unittest.main()
