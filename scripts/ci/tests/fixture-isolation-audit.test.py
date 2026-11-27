#!/usr/bin/env python3
"""Verify the fixture isolation ratchet on synthetic Rust test sources."""

from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest


ROOT = Path(__file__).resolve().parents[3]
AUDIT = ROOT / "scripts/ci/fixture-isolation-audit.py"
ALLOWLIST = "scripts/ci/fixture-isolation-allowlist.tsv"
TEST = "crates/example/tests/fixtures.rs"


class FixtureIsolationAudit(unittest.TestCase):
    def setUp(self):
        temp = TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.write(ALLOWLIST, "")

    def write(self, path, text):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def audit(self):
        return subprocess.run(
            [sys.executable, str(AUDIT), "--root", str(self.root)],
            capture_output=True, text=True, check=False,
        )

    def test_direct_operations_in_integration_and_unit_tests_are_rejected(self):
        self.write(TEST, """fn git_fixture() {
    std::process::Command::new(
        "/usr/bin/git",
    ).status();
}
""")
        self.write("crates/example/src/lib.rs", """
#[cfg(all(unix, test))]
mod unit {
    fn fixture() {
        UnixListener::bind("raw.sock");
        let root = std::env::var_os("TMPDIR");
        let other = std::env::temp_dir();
    }
}
fn runtime_only() {
    UnixStream::connect("real.sock");
}
""")
        result = self.audit()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(f"{TEST}:2: git", result.stderr)
        self.assertIn("lib.rs:5: unix-socket", result.stderr)
        self.assertIn("lib.rs:6: tmpdir", result.stderr)
        self.assertIn("lib.rs:7: tmpdir", result.stderr)
        self.assertNotIn("lib.rs:11:", result.stderr)

    def test_shared_helpers_and_comments_do_not_trigger_the_gate(self):
        self.write(TEST, """fn fixture() {
    // Command::new("git");
    let fixture = Fixture::new();
    fixture.git_command(fixture.temp_dir()).status();
    let socket = fixture.runtime_dir().join("private.sock");
    UnixListener::bind(&socket);
    UnixStream::connect(fixture.runtime_dir().join("private.sock"));
    let (_root, path) = nils_test_support::short_socket_path("short.sock");
    UnixDatagram::bind(&path);
}
""")
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stderr)
        with (self.root / TEST).open("a", encoding="utf-8") as source:
            source.write('fn other() { UnixListener::bind(&socket); }\n')
        result = self.audit()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(f"{TEST}:11: unix-socket", result.stderr)

    def test_allowlist_requires_a_reason_and_applies_to_one_site(self):
        line = 'let root = std::env::var_os("TMPDIR");'
        self.write(TEST, f"{line}\n")
        entry = f"{TEST}\ttmpdir\t{line}\t1\t"
        self.write(ALLOWLIST, f"{entry}\n")
        self.assertIn("invalid or duplicate allowlist entry", self.audit().stderr)
        self.write(ALLOWLIST, f"{entry}Exercise the inherited root.\n")
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.write(TEST, f"{line}\n{line}\n")
        result = self.audit()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(f"{TEST}:2: tmpdir", result.stderr)


if __name__ == "__main__":
    unittest.main()
