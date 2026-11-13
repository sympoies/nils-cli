#!/usr/bin/env python3
"""Synthetic portability regressions; never execute the fixture shell text."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]
AUDIT = REPO / "scripts/ci/test-shell-portability-audit.py"
OLD_WEBHOOK = '    stat -c %a "$6" > "$dir/post-mode"'
FALLBACK = """    (stat -c '%a' "$6" 2>/dev/null || stat -f '%Lp' "$6") > "$dir/post-mode" """
FALLBACK = FALLBACK.rstrip()


class TestShellPortability(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="test-shell-portability-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.write("scripts/ci/test-shell-portability-allowlist.json", "[]\n")

    def write(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def run_command(self, *args):
        return subprocess.run(args, cwd=self.root, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, check=False)

    def audit(self, root=None):
        return self.run_command(sys.executable, str(AUDIT), "--root", str(root or self.root))

    def allow(self, line, kind="fallback"):
        self.write("scripts/ci/test-shell-portability-allowlist.json", json.dumps([
            {"path": "crates/example/tests/stub.rs", "line": line,
             "kind": kind, "context": line,
             "reason": "GNU/Linux stat falls back to macOS BSD stat."}
        ]))

    def test_entrypoint_rejects_pre_3654_webhook_line(self):
        # Use the real entrypoint with a no-build verification stand-in.
        self.write("crates/example/tests/stub.rs", OLD_WEBHOOK + "\n")
        for name in ("nils-cli-checks-entrypoint.sh", "test-shell-portability-audit.py"):
            source = REPO / "scripts/ci" / name
            if source.exists():
                shutil.copyfile(source, self.root / "scripts/ci" / name)
        self.write("scripts/ci/test-env.sh", "")
        self.write("scripts/ci/nils-cli-verification.sh", "exit 0\n")
        self.run_command("git", "init", "-q")
        result = self.run_command("bash", "scripts/ci/nils-cli-checks-entrypoint.sh", "--docs-only")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("crates/example/tests/stub.rs:1", result.stdout)
        self.assertIn("stat", result.stdout)

    def test_required_gnu_forms_are_rejected_in_sources_and_fixtures(self):
        for path in ("crates/example/tests/stub.rs", "tests/fixtures/fake-gh"):
            for command in (OLD_WEBHOOK, "stat --format=%a file", "sed -i 's/a/b/' file",
                            "readlink -f file", "date -d yesterday", "grep -P pattern file", "xargs -r echo"):
                with self.subTest(path=path, command=command):
                    self.write(path, command + "\n")
                    result = self.audit()
                    self.assertEqual(result.returncode, 1, result.stdout)
                    self.assertIn(f"{path}:1:", result.stdout)
            (self.root / path).unlink()

    def test_fallback_requires_exact_allowlist(self):
        self.write("crates/example/tests/stub.rs", FALLBACK + "\n")
        self.assertEqual(self.audit().returncode, 1)
        self.allow(FALLBACK)
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.write("crates/example/tests/stub.rs", FALLBACK + "\nstat -c %a other\n")
        self.assertEqual(self.audit().returncode, 1)

    def test_classifier_input_is_explicitly_excluded(self):
        line = '    "readlink -f $(which a) $(which b)",'
        self.write("crates/example/tests/stub.rs", line + "\n")
        self.allow(line, kind="classifier-input")
        self.assertEqual(self.audit().returncode, 0)

    def test_stale_allowlist_fails(self):
        self.allow(FALLBACK)
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertIn("stale allowlist", result.stdout)

    def test_multiline_fallback_must_remain_present(self):
        line = "if stat -c '%a' file; then"
        context = line + "\n  return\nfi\nstat -f '%Lp' file"
        self.allow(line)
        rows = json.loads((self.root / "scripts/ci/test-shell-portability-allowlist.json").read_text())
        rows[0]["context"] = context
        self.write("scripts/ci/test-shell-portability-allowlist.json", json.dumps(rows))
        self.write("crates/example/tests/stub.rs", context + "\n")
        self.assertEqual(self.audit().returncode, 0)
        self.write("crates/example/tests/stub.rs", line + "\n  return\nfi\n")
        self.assertEqual(self.audit().returncode, 1)

    def test_portable_sed_suffixes_pass(self):
        self.write("tests/fixtures/stub.sh", "sed -i.bak 's/a/b/' file\nsed -i '' 's/a/b/' file\n")
        result = self.audit()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_current_tree_passes(self):
        result = self.audit(REPO)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_inline_rust_unit_tests_are_scanned(self):
        self.write("crates/example/src/lib.rs", 'const INPUT: &str = "date -d yesterday";\n')
        self.assertEqual(self.audit().returncode, 0)
        self.write("crates/example/src/lib.rs", '#[cfg(test)]\nmod tests {\n'
                   'const STUB: &str = "date -d yesterday";\n}\n')
        result = self.audit()
        self.assertEqual(result.returncode, 1)
        self.assertIn("crates/example/src/lib.rs:3:", result.stdout)


if __name__ == "__main__":
    unittest.main()
