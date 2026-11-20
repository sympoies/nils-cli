#!/usr/bin/env python3
"""Stale gate subtree regressions with synthetic systemd and cgroup state."""
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'gate_resources', Path(__file__).resolve().parents[1] / 'gate-resources.py')
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)

SLICE = 'nilsgate' + 'a' * 32 + '.slice'
SCOPE = 'tmux-spawn-fixture.scope'


class StaleSlice(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.cgroups = self.root / 'cgroups'
        self.group = f'/fixture/{SLICE}'
        self.child_group = f'{self.group}/{SCOPE}'
        self.groups = {SLICE: self.group, SCOPE: self.child_group}
        self.stopped = []
        self.owner = {'pid': 12345, 'slice': SLICE}
        self.make_cgroup(self.group, descendants=1)
        self.make_cgroup(self.child_group)

        patches = (
            patch.object(resources, 'CGROUP_ROOT', self.cgroups),
            patch.object(resources.subprocess, 'check_output', side_effect=self.show),
            patch.object(resources.subprocess, 'run', side_effect=self.stop),
            patch.object(resources.time, 'sleep', side_effect=AssertionError('stale slice blocked admission')),
        )
        for replacement in patches:
            replacement.start()
            self.addCleanup(replacement.stop)
        probe = patch.object(resources.os, 'kill', side_effect=ProcessLookupError)
        self.probe = probe.start()
        self.addCleanup(probe.stop)

    def make_cgroup(self, group, descendants=0):
        path = self.cgroups / group.lstrip('/')
        path.mkdir(parents=True, exist_ok=True)
        (path / 'cgroup.procs').write_text('')
        (path / 'cgroup.stat').write_text(f'nr_descendants {descendants}\n')
        (path / 'cgroup.events').write_text('populated 0\n')

    def show(self, command, **kwargs):
        self.assertEqual(command[:3], [resources.SYSTEMCTL, '--user', 'show'])
        unit = command[3]
        group = self.groups.get(unit)
        if group is None:
            return 'LoadState=not-found\nActiveState=inactive\nControlGroup=\nJob=\n'
        return f'Id={unit}\nLoadState=loaded\nActiveState=active\nControlGroup={group}\nJob=\n'

    def stop(self, command, **kwargs):
        self.assertEqual(command[:3], [resources.SYSTEMCTL, '--user', 'stop'])
        unit = command[-1]
        self.stopped.append(unit)
        group = self.groups.pop(unit)
        shutil.rmtree(self.cgroups / group.lstrip('/'))
        for ancestor in self.groups.values():
            if group.startswith(ancestor + '/'):
                count = sum(other.startswith(ancestor + '/') for other in self.groups.values())
                (self.cgroups / ancestor.lstrip('/') / 'cgroup.stat').write_text(
                    f'nr_descendants {count}\n')

    def test_empty_descendant_is_stopped_before_slice_and_admission(self):
        (self.root / 'gate-0.lock').write_text(json.dumps(self.owner))
        gate = resources.GateSemaphore(self.root, 1).acquire(.1)
        self.addCleanup(gate.close)
        self.assertEqual(self.stopped, [SCOPE, SLICE])

    def test_nested_units_are_stopped_from_the_leaves(self):
        inner = SLICE.removesuffix('.slice') + '-inner.slice'
        self.groups[inner] = f'{self.group}/{inner}'
        self.groups[SCOPE] = f'{self.groups[inner]}/{SCOPE}'
        shutil.rmtree(self.cgroups / self.child_group.lstrip('/'))
        self.make_cgroup(self.group, descendants=2)
        self.make_cgroup(self.groups[inner], descendants=1)
        self.make_cgroup(self.groups[SCOPE])
        self.assertTrue(resources.reap_stale_slice(self.owner))
        self.assertEqual(self.stopped, [SCOPE, inner, SLICE])

    def test_nonempty_descendant_is_refused(self):
        for group in (self.group, self.child_group):
            (self.cgroups / group.lstrip('/') / 'cgroup.events').write_text('populated 1\n')
        (self.cgroups / self.child_group.lstrip('/') / 'cgroup.procs').write_text('42\n')
        self.assertFalse(resources.reap_stale_slice(self.owner))
        self.assertEqual(self.stopped, [])

    def test_live_holder_is_refused(self):
        self.probe.side_effect = None
        self.assertFalse(resources.reap_stale_slice(self.owner))
        self.assertEqual(self.stopped, [])

    def test_unrecorded_slice_is_refused(self):
        self.assertFalse(resources.reap_stale_slice({'pid': self.owner['pid']}))
        self.assertEqual(self.stopped, [])

    def test_descendant_without_matching_systemd_unit_is_refused(self):
        del self.groups[SCOPE]
        self.assertFalse(resources.reap_stale_slice(self.owner))
        self.assertEqual(self.stopped, [])

    def test_restartable_service_is_refused(self):
        service = 'restartable-fixture.service'
        service_group = f'{self.group}/{service}'
        self.groups[service] = service_group
        del self.groups[SCOPE]
        service_path = self.cgroups / service_group.lstrip('/')
        (self.cgroups / self.child_group.lstrip('/')).rename(service_path)
        events = self.cgroups / self.group.lstrip('/') / 'cgroup.events'
        original_read = Path.read_text
        reads = 0

        def read(path, *args, **kwargs):
            nonlocal reads
            snapshot = original_read(path, *args, **kwargs)
            if path == events:
                reads += 1
                if reads == 2:
                    # Simulate an automatic restart after the last empty snapshot.
                    (service_path / 'cgroup.procs').write_text('42\n')
                    (service_path / 'cgroup.events').write_text('populated 1\n')
                    events.write_text('populated 1\n')
            return snapshot

        with patch.object(Path, 'read_text', read):
            self.assertFalse(resources.reap_stale_slice(self.owner))
        self.assertEqual(self.stopped, [])

    def test_subtree_population_is_rechecked_before_stopping_descendant(self):
        reads = 0

        def show(command, **kwargs):
            nonlocal reads
            if command[3] == SCOPE:
                reads += 1
                if reads == 2:
                    events = self.cgroups / self.group.lstrip('/') / 'cgroup.events'
                    events.write_text('populated 1\n')
            return self.show(command, **kwargs)

        with patch.object(resources.subprocess, 'check_output', side_effect=show):
            self.assertFalse(resources.reap_stale_slice(self.owner))
        self.assertEqual(reads, 2)
        self.assertEqual(self.stopped, [])

    def test_slice_is_rechecked_after_stopping_descendant(self):
        def stop(command, **kwargs):
            self.stop(command, **kwargs)
            (self.cgroups / self.group.lstrip('/') / 'cgroup.procs').write_text('42\n')

        with patch.object(resources.subprocess, 'run', side_effect=stop):
            self.assertFalse(resources.reap_stale_slice(self.owner))
        self.assertEqual(self.stopped, [SCOPE])

    def test_changed_slice_or_descendant_cgroup_path_is_refused(self):
        for changed in (SLICE, SCOPE):
            with self.subTest(unit=changed):
                original = self.groups[changed]
                moved = f'/other/{changed}'
                self.make_cgroup(moved, descendants=int(changed == SLICE))
                reads = 0

                def show(command, **kwargs):
                    nonlocal reads
                    if command[3] == changed:
                        reads += 1
                        if reads > 1:
                            self.groups[changed] = moved
                    return self.show(command, **kwargs)

                with patch.object(resources.subprocess, 'check_output', side_effect=show):
                    self.assertFalse(resources.reap_stale_slice(self.owner))
                self.assertGreater(reads, 1)
                self.assertEqual(self.stopped, [])
                self.groups[changed] = original


if __name__ == '__main__':
    unittest.main()
