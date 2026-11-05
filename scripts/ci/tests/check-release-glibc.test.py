#!/usr/bin/env python3
"""Synthetic ELF version-output tests for the Linux release gate."""

import contextlib
import io
from pathlib import Path
import runpy
import subprocess
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[3]
CHECK = runpy.run_path(str(ROOT / "scripts/ci/check-release-glibc.py"))["check_binaries"]


class ReleaseGlibcTests(unittest.TestCase):
    def check(self, outputs, binaries):
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch("subprocess.run", side_effect=outputs) as readelf:
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                code = CHECK(binaries)
        self.assertEqual(readelf.call_count, len(binaries))
        for call, binary in zip(readelf.call_args_list, binaries):
            self.assertEqual(call.args[0], ["readelf", "--version-info", "--wide", binary])
        return code, stdout.getvalue(), stderr.getvalue()

    @staticmethod
    def output(versions):
        return subprocess.CompletedProcess(
            [], 0, "\n".join(f"Name: {version}  Flags: none" for version in versions), "",
        )

    def test_accepts_baseline_older_and_static_binaries(self):
        code, stdout, stderr = self.check([
            self.output(["GLIBC_2.2.5", "GLIBC_2.9", "GLIBC_2.39", "GCC_4.2.0"]),
            self.output([]),
        ], ["baseline-bin", "static-bin"])
        self.assertEqual(code, 0)
        self.assertIn("ok: baseline-bin", stdout)
        self.assertIn("ok: static-bin", stdout)
        self.assertEqual(stderr, "")

    def test_reports_all_offending_binaries_and_numeric_versions(self):
        code, _, stderr = self.check([
            self.output(["GLIBC_2.43", "GLIBC_2.43"]),
            self.output(["GLIBC_2.100", "GLIBC_2.40"]),
        ], ["image-processing", "another-bin"])
        self.assertEqual(code, 1)
        self.assertEqual(stderr.count("image-processing: requires GLIBC_2.43"), 1)
        self.assertIn("another-bin: requires GLIBC_2.40", stderr)
        self.assertIn("another-bin: requires GLIBC_2.100", stderr)
        self.assertIn("supported GLIBC_2.39", stderr)

    def test_inspection_errors_fail_closed(self):
        for error in [FileNotFoundError("readelf"), subprocess.CalledProcessError(1, "readelf")]:
            with self.subTest(error=error):
                code, _, stderr = self.check([error], ["unreadable-bin"])
                self.assertEqual(code, 1)
                self.assertIn("unreadable-bin: cannot inspect ELF symbol versions", stderr)

    def test_release_and_cache_pin_x86_64_runner(self):
        for name in ["release.yml", "release-cache.yml"]:
            workflow = (ROOT / ".github/workflows" / name).read_text()
            self.assertIn(
                "- runs_on: ubuntu-24.04\n            target: x86_64-unknown-linux-gnu\n",
                workflow,
            )

    def test_linux_release_gate_runs_before_artifact_upload(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        step = workflow.index("      - name: Check Linux glibc requirements\n")
        upload = workflow.index("      - name: Upload artifact\n")
        self.assertLess(step, upload)
        self.assertIn("        if: runner.os == 'Linux'\n", workflow[step:upload])
        self.assertIn(
            "python3 scripts/ci/check-release-glibc.py "
            '"dist/nils-cli-${{ github.ref_name }}-${{ matrix.target }}/bin/"*',
            workflow[step:upload],
        )


if __name__ == "__main__":
    unittest.main()
