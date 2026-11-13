#!/usr/bin/env python3
"""Executing-cgroup receipt regressions using ordinary temporary files only."""
import importlib.util
import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('gate_resources', ROOT / 'scripts/ci/gate-resources.py')
resources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resources)
GIB = resources.GIB


class EffectiveLimits(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tree = self.root / 'cgroup'
        self.tree.mkdir()
        self.proc = self.root / 'self.cgroup'
        self.proc.write_text('0::/gate/workload\n')
        self.group('gate', 16 * GIB, 0)
        self.group('gate/workload', 16 * GIB, 0)
        self.receipt = self.root / 'receipt.json'

    def group(self, name, memory, swap):
        directory = self.tree / name
        directory.mkdir(parents=True, exist_ok=True)
        for filename, value in [('memory.max', memory), ('memory.swap.max', swap)]:
            (directory / filename).write_text(str(value) + '\n')

    def run_inside(self, platform='linux'):
        env = {'NILS_CLI_GATE_SUPERVISOR_PID': str(os.getpid()),
               'NILS_CLI_GATE_SLICE': 'fixture.slice',
               'NILS_CLI_GATE_MEMORY_MAX_GIB': '16',
               'NILS_CLI_GATE_RECEIPT': str(self.receipt)}
        def workload(command):
            self.assertEqual(command, ['fixture-workload'])
            receipt = json.loads(self.receipt.read_text()) if self.receipt.exists() else {}
            self.assertIn('effective', receipt, 'workload must see effective limits in its receipt')
            return 7
        with patch.dict(os.environ, env, clear=True), \
             patch.object(resources.sys, 'platform', platform), \
             patch.object(resources, 'SELF_CGROUP', self.proc, create=True), \
             patch.object(resources, 'CGROUP_ROOT', self.tree, create=True), \
             patch.object(resources.subprocess, 'run'), \
             contextlib.redirect_stderr(io.StringIO()), \
             patch.object(resources.subprocess, 'call', side_effect=workload):
            self.assertEqual(resources.main(['--inside', '--', 'fixture-workload']), 7)
        return json.loads(self.receipt.read_text())

    def test_equal_executing_limits(self):
        # A launching cgroup is deliberately different; only /self/cgroup wins.
        self.group('launcher', 2 * GIB, 'max')
        receipt = self.run_inside()
        self.assertEqual(receipt['requested'], {'memory_max_bytes': 16 * GIB, 'memory_swap_max_bytes': 0})
        self.assertEqual(receipt['effective']['memory_max']['bytes'], 16 * GIB)
        self.assertEqual(receipt['effective']['memory_swap_max']['bytes'], 0)
        self.assertFalse(receipt['mismatch'])

    def test_lower_executing_limit(self):
        self.group('gate/workload', 8 * GIB, 0)
        receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_max']['bytes'], 8 * GIB)
        self.assertTrue(receipt['mismatch'])

    def test_ancestor_constrains_unlimited_workload(self):
        self.group('gate', 4 * GIB, 0)
        self.group('gate/workload', 'max', 'max')
        receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_max']['bytes'], 4 * GIB)
        self.assertEqual(receipt['effective']['memory_max']['limiting_ancestor_levels'], 1)
        self.assertEqual(receipt['effective']['memory_swap_max']['bytes'], 0)
        self.assertTrue(receipt['mismatch'])

    def test_unlimited(self):
        self.group('gate', 'max', 'max')
        self.group('gate/workload', 'max', 'max')
        receipt = self.run_inside()
        for value in receipt['effective'].values():
            self.assertEqual(value['status'], 'unlimited')
            self.assertIsNone(value['bytes'])
        self.assertTrue(receipt['mismatch'])

    def test_missing_file_is_unavailable_not_requested(self):
        (self.tree / 'gate/workload/memory.max').unlink()
        receipt = self.run_inside()
        value = receipt['effective']['memory_max']
        self.assertEqual(value['status'], 'unavailable')
        self.assertEqual(value['reason'], 'file_missing')
        self.assertIsNone(value['bytes'])
        self.assertIsNone(receipt['mismatch'])
        self.assertEqual(receipt['effective']['memory_swap_max']['bytes'], 0)

    def test_mounted_delegated_root_can_constrain_workload(self):
        self.group('', 2 * GIB, 0)
        receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_max']['bytes'], 2 * GIB)
        self.assertEqual(receipt['effective']['memory_max']['limiting_ancestor_levels'], 2)
        self.assertTrue(receipt['mismatch'])

    def test_unreadable_ancestor_is_explicit(self):
        original = Path.read_text
        def read(path, *args, **kwargs):
            if path == self.tree / 'gate/memory.swap.max':
                raise PermissionError('fixture')
            return original(path, *args, **kwargs)
        with patch.object(Path, 'read_text', read):
            receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_swap_max']['reason'], 'file_unreadable')
        self.assertIsNone(receipt['mismatch'])

    def test_v1_is_explicit(self):
        self.proc.write_text('4:memory:/gate/workload\n')
        receipt = self.run_inside()
        self.assertEqual(receipt['cgroup_version'], 1)
        for value in receipt['effective'].values():
            self.assertEqual(value['reason'], 'cgroup_v1')
            self.assertIsNone(value['bytes'])
        self.assertIsNone(receipt['mismatch'])

    def test_missing_membership_is_explicit(self):
        self.proc.unlink()
        receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_max']['reason'], 'membership_missing')
        self.assertIsNone(receipt['cgroup_version'])
        self.assertIsNone(receipt['mismatch'])

    def test_malformed_membership_is_not_guessed_as_v1(self):
        self.proc.write_text('')
        receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_max']['reason'], 'membership_invalid')
        self.assertIsNone(receipt['cgroup_version'])

    def test_malformed_limit_is_unavailable(self):
        self.group('gate/workload', 'invalid', 0)
        receipt = self.run_inside()
        self.assertEqual(receipt['effective']['memory_max']['reason'], 'invalid_value')
        self.assertIsNone(receipt['mismatch'])

    def test_non_linux_is_explicit(self):
        receipt = self.run_inside('darwin')
        self.assertEqual(receipt['effective']['memory_max']['reason'], 'non_linux')
        self.assertIsNone(receipt['mismatch'])


if __name__ == '__main__':
    unittest.main()
