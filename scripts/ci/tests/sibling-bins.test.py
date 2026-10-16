#!/usr/bin/env python3
"""Regressions for the sibling-binary scan behind package-scoped local-fast builds."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
from sibling_bins import sibling_packages


def package(root, name, bins=()):
    targets = [{'kind': ['lib'], 'name': name.replace('-', '_')}]
    targets += [{'kind': ['bin'], 'name': binary} for binary in bins]
    return {'name': name, 'manifest_path': str(root / name / 'Cargo.toml'), 'targets': targets}


class SiblingScanTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.packages = [
            package(self.root, 'consumer', bins=['consumer']),
            package(self.root, 'helper-pkg', bins=['helper']),
            package(self.root, 'other-pkg', bins=['other']),
        ]

    def source(self, name, relative, text):
        path = self.root / name / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def test_resolved_binary_of_an_unselected_package_is_a_sibling(self):
        # A stale or missing helper in target/<profile> is what a package-scoped
        # run would otherwise trust; the scan must name its owner for the build.
        self.source('consumer', 'src/tests/fixture.rs',
                    'let helper = nils_test_support::bin::resolve("helper");\n')
        self.assertEqual(sibling_packages(self.packages, {'consumer'}),
                         {'helper-pkg': {'helper resolved by consumer'}})

    def test_every_resolver_form_is_recognized(self):
        self.source('consumer', 'tests/integration/cli.rs', '''
if let Some(path) =
    nils_test_support::bin::sibling_or_skip(
        "helper",
        "helper-pkg",
    )
{}
let other = required_sibling("other", "other-pkg");
run_resolved("helper", &["--version"], &options);
''')
        self.assertEqual(sibling_packages(self.packages, {'consumer'}),
                         {'helper-pkg': {'helper resolved by consumer'},
                          'other-pkg': {'other resolved by consumer'}})

    def test_own_selected_and_unknown_binaries_are_not_siblings(self):
        self.source('consumer', 'tests/cli.rs', '''
resolve("consumer");
resolve("helper");
resolve_optional("not-a-workspace-binary");
''')
        self.assertEqual(sibling_packages(self.packages, {'consumer', 'helper-pkg'}), {})

    def test_unselected_package_sources_are_not_scanned(self):
        self.source('other-pkg', 'tests/cli.rs', 'resolve("helper");\n')
        self.assertEqual(sibling_packages(self.packages, {'consumer'}), {})


if __name__ == '__main__':
    unittest.main()
