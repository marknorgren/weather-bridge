#!/usr/bin/env python3
"""Regression tests for smoke-test process startup diagnostics."""

import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest

from scripts import smoke


ROOT = Path(__file__).resolve().parents[1]
SMOKE = ROOT / "scripts/smoke.py"


def failing_binary(path: Path, marker: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        textwrap.dedent(
            f"""\
            #!{sys.executable}
            import sys
            print({marker!r}, file=sys.stderr)
            raise SystemExit(23)
            """
        )
    )
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class SmokeStartupTests(unittest.TestCase):
    def run_smoke(self, *arguments: str, env: dict[str, str] | None = None):
        return subprocess.run(
            [sys.executable, str(SMOKE), *arguments],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_developer_reference_accepts_custom_https_docs(self):
        smoke.Smoke.check_docs_links('<a href="https://docs.example.com/fork/rest.html">REST reference</a>')

    def test_developer_reference_rejects_missing_or_unsafe_links(self):
        for page in ["REST reference", '<a href="http://docs.example.com/rest.html">REST reference</a>']:
            with self.subTest(page=page), self.assertRaises(AssertionError):
                smoke.Smoke.check_docs_links(page)

    def test_explicit_binary_reports_bounded_startup_log(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "broken-weather-bridge"
            marker = "explicit binary startup failed deliberately"
            failing_binary(binary, "discarded-log-line-" * 600 + "\n" + marker)

            result = self.run_smoke("--spawn", "--binary", str(binary))

        self.assertEqual(result.returncode, 1, result)
        self.assertIn("startup phase failed", result.stderr)
        self.assertIn(marker, result.stderr)
        self.assertIn("startup log truncated", result.stderr)
        self.assertLess(len(result.stderr.encode()), 8_500)
        self.assertNotIn("Traceback", result.stderr)

    def test_cargo_target_dir_selects_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "custom-target"
            marker = "custom target binary selected"
            failing_binary(target / "debug/weather-bridge", marker)
            env = {**os.environ, "CARGO_TARGET_DIR": str(target)}

            result = self.run_smoke("--spawn", env=env)

        self.assertEqual(result.returncode, 1, result)
        self.assertIn("startup phase failed", result.stderr)
        self.assertIn(marker, result.stderr)

    def test_spawned_server_cleanup_terminates_child(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "waiting-weather-bridge"
            binary.write_text(
                f"#!{sys.executable}\nimport time\ntime.sleep(60)\n"
            )
            binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
            server = smoke.SpawnedServer(binary, 1)

            server.close()

        self.assertIsNotNone(server.process.poll())


if __name__ == "__main__":
    unittest.main()
