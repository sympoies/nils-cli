#!/usr/bin/env python3
"""Resource admission regressions; fake workloads, no compiler or systemd."""
import importlib.util
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
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('gate_resources', ROOT / 'scripts/ci/gate-resources.py')
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)


def hold_slot(directory, started):
    lease = resources.GateSemaphore(Path(directory), 1).acquire(5)
    started.set()
    time.sleep(30)
    lease.close()


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
                with self.assertRaisesRegex(TimeoutError, 'holders: pid=.*since='):
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
                with self.assertRaisesRegex(TimeoutError, 'timed out'):
                    resources.GateSemaphore(root, 2).acquire(.1)
            finally:
                ticket.close()
            lease = resources.GateSemaphore(root, 2).acquire(.5)
            lease.close()

    def test_insufficient_memory_never_admits_work(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(TimeoutError, 'waiting for memory'):
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
            with self.assertRaisesRegex(TimeoutError, 'holders: pid='):
                resources.GateSemaphore(root, 1, quiescent=lambda _: False).acquire(.1)
            lease = resources.GateSemaphore(root, 1, quiescent=lambda _: True).acquire(.5)
            lease.close()

    def test_gate_holds_admission_until_slice_cleanup_returns(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            env = {'XDG_STATE_HOME': directory, 'NILS_CLI_RUNNER_MAX': '2'}
            def cleanup(*args, **kwargs):
                with self.assertRaises(TimeoutError):
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
        shutil.copy(ROOT / 'scripts/ci/lib/doc_classify.py', ci / 'lib/doc_classify.py')
        verify = root / '.agents/skills/project-verify-required-checks/scripts'
        verify.mkdir(parents=True)
        (verify / 'project-verify-required-checks.sh').write_text('touch "$DOCS_CALLED"\n')
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
                'print(json.dumps({"packages":[{"name":"example", "manifest_path":'
                'str(Path.cwd()/"crates/example/Cargo.toml"), "targets":[]}]}))\n')
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
