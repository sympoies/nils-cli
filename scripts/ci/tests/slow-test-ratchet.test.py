#!/usr/bin/env python3
"""Slow-test ratchet: new names are reported, baselined ones are not, baseline only shrinks."""
import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    'ratchet', Path(__file__).resolve().parents[1] / 'slow-test-ratchet.py')
ratchet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ratchet)


def junit(cases):
    body = ''.join(f'<testcase name="{n}" classname="{c}" time="{t}"></testcase>' for c, n, t in cases)
    handle = tempfile.NamedTemporaryFile('w', suffix='.xml', delete=False)
    handle.write(f'<testsuites><testsuite name="s">{body}</testsuite></testsuites>')
    handle.close()
    return handle.name


class RatchetTests(unittest.TestCase):
    def test_names_unbaselined_slow_test_only(self):
        found = ratchet.slow_tests(junit([('a', 'fast', 5.9), ('a', 'new_slow', 10.0), ('a', 'old_slow', 40)]), 6)
        report, new = ratchet.render(found, {('a', 'old_slow')}, 10)
        self.assertEqual(new, [('a', 'new_slow')])
        self.assertIn('new_slow', report)
        self.assertNotIn('old_slow', report)
        self.assertNotIn('fast', report)

    def test_baselined_only_run_is_clean_and_reports_stale_rows(self):
        found = ratchet.slow_tests(junit([('a', 'old_slow', 12)]), 6)
        report, new = ratchet.render(found, {('a', 'old_slow'), ('a', 'fixed')}, 10)
        self.assertEqual(new, [])
        self.assertIn('1 baseline row(s)', report)

    def test_baselined_near_threshold_test_is_not_named_when_it_slows_down(self):
        found = ratchet.slow_tests(junit([('a', 'borderline', 15.4), ('a', 'below', 7.0)]), 6)
        report, new = ratchet.render(found, {('a', 'borderline'), ('a', 'below')}, 10)
        self.assertEqual(new, [])
        self.assertNotIn('borderline', report)

    def test_unbaselined_test_between_floor_and_threshold_is_not_named(self):
        found = ratchet.slow_tests(junit([('a', 'mid', 8.0)]), 6)
        self.assertEqual(ratchet.render(found, set(), 10)[1], [])

    def test_baseline_may_shrink_or_swap_but_not_grow(self):
        x, y = ('a', 'x'), ('a', 'y')
        self.assertEqual(ratchet.baseline_additions({x}, {x, y}), [y])
        self.assertEqual(ratchet.baseline_additions({x}, {y}), [])
        self.assertEqual(ratchet.baseline_additions({x}, set()), [])


if __name__ == '__main__':
    unittest.main()
