#!/usr/bin/env python3
"""Cargo-free assertion import audit regressions."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
AUDIT = ROOT / 'scripts/ci/assert-import-audit.py'


class AuditTests(unittest.TestCase):
    def audit(self, files):
        with tempfile.TemporaryDirectory(prefix='assert-import-audit-') as directory:
            root = Path(directory)
            for name, source in files.items():
                path = root / 'crates/demo' / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(source)
            # No executable (including cargo) can be found by the audit.
            return subprocess.run([sys.executable, str(AUDIT), '--root', str(root)],
                                  env=dict(os.environ, PATH=''), text=True,
                                  capture_output=True)

    def test_check_chains_run_audit(self):
        for script in ('scripts/ci/nils-cli-local-fast.sh',
                       'scripts/ci/nils-cli-scoped-checks.sh',
                       '.agents/skills/project-verify-required-checks/scripts/'
                       'project-verify-required-checks.sh'):
            with self.subTest(script=script):
                source = (ROOT / script).read_text()
                call = 'run python3 scripts/ci/assert-import-audit.py'
                self.assertTrue(call in source, f'{script} must reject ambiguous child imports')
                self.assertLess(source.index(call), source.index('run cargo fmt'))
                self.assertIn('run python3 scripts/ci/tests/assert-import-audit.test.py', source)

    def test_inline_children_and_macro_specific_imports(self):
        result = self.audit({'src/lib.rs': '''
#[cfg(test)]
mod tests {
    use pretty_assertions::{assert_eq, assert_ne};
    mod bad {
        use super::*;
        #[test] fn check() { assert_eq!(1, 1); assert_ne!(1, 2); }
    }
    mod partial {
        use super::*;
        use pretty_assertions::assert_eq;
        #[test] fn check() { assert_eq!(1, 1); assert_ne!(1, 2); }
    }
    mod clean {
        use super::*;
        use pretty_assertions::{assert_eq, assert_ne};
        #[test] fn check() { assert_eq!(1, 1); assert_ne!(1, 2); }
    }
}
'''})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('crates/demo/src/lib.rs:7:', result.stdout)
        self.assertEqual(result.stdout.count('explicitly import pretty_assertions::assert_eq'), 1)
        self.assertEqual(result.stdout.count('explicitly import pretty_assertions::assert_ne'), 2)

    def test_external_children_path_and_include(self):
        result = self.audit({
            'src/lib.rs': '#[cfg(test)] mod tests;',
            'src/tests.rs': '''
use pretty_assertions::{assert_eq, assert_ne};
mod extracted;
#[path = "custom.rs"] mod custom;
mod inline { include!("included.rs"); }
mod nested { #[path = "alternate.rs"] mod alternate; }
''',
            'src/tests/extracted.rs': 'use super::*; fn check() { assert_eq!(1, 1); }',
            'src/custom.rs': 'use super::*; fn check() { assert_ne!(1, 2); }',
            'src/included.rs': 'use super::*; fn check() { assert_eq!(1, 1); }',
            'src/tests/nested/alternate.rs': 'fn check() { assert_eq!(1, 1); }',
            'tests/integration.rs': '''
use pretty_assertions::assert_ne;
mod children;
''',
            'tests/integration/children/mod.rs': 'use super::*; fn check() { assert_ne!(1, 2); }',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        for path in ('src/tests/extracted.rs', 'src/custom.rs', 'src/included.rs',
                     'tests/integration/children/mod.rs'):
            self.assertIn(f'crates/demo/{path}:', result.stdout)
        self.assertEqual(result.stdout.count('explicitly import'), 4)

    def test_include_shares_scope_with_parent_and_child(self):
        result = self.audit({
            'tests/integration.rs': 'include!("imports.rs"); mod child;',
            'tests/imports.rs': 'use pretty_assertions::assert_eq;',
            'tests/integration/child.rs': '''
use super::*;
include!("explicit.rs");
fn check() { assert_eq!(1, 1); }
mod grandchild { use super::*; fn check() { assert_eq!(1, 1); } }
''',
            'tests/integration/explicit.rs': 'use pretty_assertions::assert_eq;',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count('explicitly import'), 1)
        self.assertIn('crates/demo/tests/integration/child.rs:5:', result.stdout)

    def test_clean_controls_and_lexical_noise(self):
        result = self.audit({'tests/integration.rs': '''
use pretty_assertions::{assert_eq, assert_ne};
mod no_glob { fn check() { assert_eq!(1, 1); } }
mod no_call { use super::*; }
mod sibling {
    use pretty_assertions::assert_eq;
}
mod clean {
    use super::*;
    use ::pretty_assertions::{assert_eq, assert_ne};
    fn check() { assert_eq!(1, 1); assert_ne!(1, 2); }
}
mod qualified {
    use super::*;
    fn check() { std::assert_eq!(1, 1); pretty_assertions::assert_ne!(1, 2); }
    // assert_eq!(0, 1); mod fake { use super::*; }
    /* nested /* assert_ne!(0, 0); */ comment */
    const TEXT: &str = r###"assert_eq!(1, 2); }"###;
}
mod parent_without_import {
    mod child { use super::*; fn check() { assert_eq!(1, 1); assert_ne!(1, 2); } }
}
mod unrelated_parent_macro {
    use pretty_assertions::assert_eq;
    mod child { use super::*; fn check() { assert_ne!(1, 2); } }
}
mod renamed {
    use pretty_assertions::{assert_eq as compare, assert_ne as different};
    mod child { use super::*; fn check() { assert_eq!(1, 1); assert_ne!(1, 2); } }
}
'''})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_disjoint_cfg_imports_do_not_exempt_calls(self):
        result = self.audit({'tests/integration.rs': '''
use pretty_assertions::{assert_eq, assert_ne};
mod child {
    use super::*;
    #[cfg(target_os = "linux")]
    use pretty_assertions::{assert_eq, assert_ne};
    #[cfg(target_os = "macos")]
    #[test] fn check() { assert_eq!(1, 1); assert_ne!(1, 2); }
}
'''})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count('explicitly import'), 2)

    def test_non_rs_literal_include_and_cycle(self):
        result = self.audit({
            'tests/integration.rs': '''
use pretty_assertions::assert_eq;
mod child { include!("../fragments/assertions.inc"); }
''',
            'fragments/assertions.inc': '''
use super::*;
include!("assertions.inc");
fn check() { assert_eq!(1, 1); }
''',
        })
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('crates/demo/fragments/assertions.inc:4:', result.stdout)
        self.assertEqual(result.stdout.count('explicitly import'), 1)

    def test_literal_include_outside_crates_is_not_followed(self):
        result = self.audit({
            'tests/integration.rs': '''
use pretty_assertions::assert_eq;
mod child { include!("../../../outside.inc"); }
''',
            '../../outside.inc': 'use super::*; fn check() { assert_eq!(1, 1); }',
        })
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_raw_c_string_does_not_hide_child(self):
        result = self.audit({'tests/integration.rs': '''
use pretty_assertions::assert_eq;
const TEXT: &std::ffi::CStr = cr#"x " }"#;
mod child { use super::*; fn check() { assert_eq!(1, 1); } }
'''})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn('crates/demo/tests/integration.rs:4:', result.stdout)
        self.assertEqual(result.stdout.count('explicitly import'), 1)

    def test_multiline_grouped_imports_and_platform_gates(self):
        result = self.audit({'src/lib.rs': '''
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use pretty_assertions::{
        self, assert_eq,
    };
    #[cfg(target_os = "macos")]
    mod child {
        use super::{*};
        fn check() { assert_eq!(1, 1); }
    }
}
'''})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(result.stdout.count('explicitly import'), 1)


if __name__ == '__main__':
    unittest.main()
