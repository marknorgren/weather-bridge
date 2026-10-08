#!/usr/bin/env python3
"""Focused regression tests for maintained Markdown link validation."""

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/build-docs.py"


class DocumentationLinkTests(unittest.TestCase):
    def run_checker(self, root: Path):
        script = root / "scripts/build-docs.py"
        script.parent.mkdir(parents=True)
        shutil.copyfile(SCRIPT, script)
        return subprocess.run(
            [sys.executable, str(script), "--check-links"],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_valid_discovered_guides_pass_without_site_build_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text("[Development](docs/development.md)\n")
            (root / "docs").mkdir()
            (root / "docs/development.md").write_text("[Project](../README.md)\n")

            result = self.run_checker(root)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Checked local links in 2 Markdown files", result.stdout)

    def test_broken_link_in_discovered_guide_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text("Project guide\n")
            (root / "docs").mkdir()
            (root / "docs/development.md").write_text("[Missing](missing-guide.md)\n")

            result = self.run_checker(root)
            built_site = (root / "target").exists()

        self.assertEqual(result.returncode, 2, result)
        self.assertIn(
            "docs/development.md: missing link target: missing-guide.md",
            result.stderr,
        )
        self.assertFalse(built_site)


if __name__ == "__main__":
    unittest.main()
