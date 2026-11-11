#!/usr/bin/env python3
"""Partitioned coverage merge: union of covered lines and the line floor."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('merge_lcov', ROOT / 'scripts/ci/merge-lcov.py')
merge_lcov = importlib.util.module_from_spec(spec)
spec.loader.exec_module(merge_lcov)

PART_ONE = 'SF:src/a.rs\nDA:1,1\nDA:2,0\nDA:3,0\nLF:3\nLH:1\nend_of_record\n'
PART_TWO = ('SF:src/a.rs\nDA:1,0\nDA:2,4\nDA:3,0\nLF:3\nLH:1\nend_of_record\n'
            'SF:src/b.rs\nDA:7,0\nLF:1\nLH:0\nend_of_record\n')


class MergeLcovTests(unittest.TestCase):
    def merge(self, floor):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'one.info').write_text(PART_ONE)
            (root / 'two.info').write_text(PART_TWO)
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                code = merge_lcov.main([str(root / 'one.info'), str(root / 'two.info'),
                                        '--output', str(root / 'merged.info'),
                                        '--fail-under-lines', str(floor)])
            return code, (root / 'merged.info').read_text()

    def test_a_line_hit_by_any_partition_counts_once(self):
        code, merged = self.merge(50)
        self.assertEqual(code, 0)
        self.assertEqual(merged, 'SF:src/a.rs\nDA:1,1\nDA:2,4\nDA:3,0\nLF:3\nLH:2\nend_of_record\n'
                                 'SF:src/b.rs\nDA:7,0\nLF:1\nLH:0\nend_of_record\n')

    def test_floor_is_checked_on_the_merged_percentage(self):
        # 2 of 4 lines hit: the floor passes at 50 and fails above it, although
        # each partition alone reports a different, lower percentage.
        self.assertEqual(self.merge(50)[0], 0)
        self.assertEqual(self.merge(50.01)[0], 1)


if __name__ == '__main__':
    unittest.main()
