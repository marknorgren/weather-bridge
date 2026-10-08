#!/usr/bin/env python3
"""Focused tests for prerequisite reporting."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from scripts import doctor


class DoctorTests(unittest.TestCase):
    def test_scopes_are_separate_and_missing_tools_have_exact_remedies(self):
        with mock.patch.object(doctor.shutil, "which", return_value=None):
            run = doctor.check_scope("run")
            docs = doctor.check_scope("docs")

        self.assertEqual({result.name for result in run}, {"cargo", "rustc"})
        self.assertIn("rustup toolchain install", "\n".join(r.remedy for r in run))
        self.assertIn("likec4", {result.name for result in docs})
        self.assertIn("playwright browsers", {result.name for result in docs})
        self.assertTrue(all(result.remedy for result in run + docs if not result.ok))

    def test_contributor_scope_includes_runtime_prerequisites(self):
        with mock.patch.object(doctor.shutil, "which", return_value=None):
            contributor = doctor.check_scope("contributor")
        names = {result.name for result in contributor}
        self.assertTrue({"cargo", "rustc", "node", "pnpm", "just"}.issubset(names))

    def test_setup_scope_checks_tools_without_requiring_node_modules(self):
        with mock.patch.object(doctor.shutil, "which", return_value=None):
            setup = doctor.check_scope("setup")
        names = {result.name for result in setup}
        self.assertEqual(names, {"cargo", "rustc", "node", "pnpm", "just"})
        self.assertNotIn("frontend dependencies", names)

    def test_docs_reject_stale_but_runnable_tool_versions(self):
        results = {
            "node": doctor.Result("node", True, "v22.17.0"),
            "likec4": doctor.Result("likec4", True, "likec4 1.59.3"),
            "playwright": doctor.Result("playwright", True, "1.61.0"),
            "playwright browsers": doctor.Result("playwright browsers", True, "installed"),
        }
        with mock.patch.object(
            doctor, "command_result", side_effect=lambda name, *_args: results[name]
        ):
            docs = {result.name: result for result in doctor.docs_checks()}

        self.assertFalse(docs["node"].ok)
        self.assertFalse(docs["likec4"].ok)
        self.assertFalse(docs["playwright"].ok)
        self.assertTrue(docs["playwright browsers"].ok)

    def test_docs_accept_required_tool_versions_and_browsers(self):
        results = {
            "node": doctor.Result("node", True, "v22.18.0"),
            "likec4": doctor.Result("likec4", True, "likec4 1.59.4"),
            "playwright": doctor.Result("playwright", True, "1.62.1"),
            "playwright browsers": doctor.Result(
                "playwright browsers", True, "Chromium and WebKit installed"
            ),
        }
        with mock.patch.object(
            doctor, "command_result", side_effect=lambda name, *_args: results[name]
        ):
            docs = doctor.docs_checks()
        self.assertTrue(all(result.ok for result in docs), docs)

    def test_zig_remedy_creates_a_discoverable_local_wrapper(self):
        self.assertIn("ziglang==0.15.2", doctor.ZIG_REMEDY)
        self.assertIn("-m ziglang", doctor.ZIG_REMEDY)
        self.assertIn("target/dev-tools/zig/bin/zig", doctor.ZIG_REMEDY)
        self.assertIn("chmod +x", doctor.ZIG_REMEDY)

    def test_setup_recipe_checks_install_tools_before_frozen_install(self):
        justfile = (doctor.ROOT / "justfile").read_text()
        setup = justfile.split("setup:", 1)[1].split("\n\n", 1)[0]
        install = "pnpm install --frozen-lockfile"
        self.assertIn("scripts/doctor.py setup", setup)
        self.assertLess(setup.index("scripts/doctor.py setup"), setup.index(install))
        self.assertLess(setup.index(install), setup.index("scripts/doctor.py contributor"))

    def test_pnpm_older_than_major_11_is_rejected(self):
        with mock.patch.object(
            doctor, "command_result", return_value=doctor.Result("pnpm", True, "10.33.0")
        ):
            self.assertFalse(doctor.pnpm_check().ok)
        with mock.patch.object(
            doctor, "command_result", return_value=doctor.Result("pnpm", True, "11.25.0")
        ):
            self.assertTrue(doctor.pnpm_check().ok)

    def test_locked_versions_read_pnpm_importer_and_drop_peer_suffixes(self):
        lockfile = """lockfileVersion: '9.0'

importers:

  .:
    dependencies:
      openapi-fetch:
        specifier: 0.17.0
        version: 0.17.0
    devDependencies:
      '@playwright/test':
        specifier: 1.58.2
        version: 1.58.2
      openapi-typescript:
        specifier: 7.13.0
        version: 7.13.0(typescript@5.9.3)

packages:

  openapi-fetch@0.17.0:
    resolution: {integrity: sha512-x}
"""
        self.assertEqual(
            doctor.locked_versions(lockfile),
            {
                "openapi-fetch": "0.17.0",
                "@playwright/test": "1.58.2",
                "openapi-typescript": "7.13.0",
            },
        )

    def test_frontend_dependencies_reject_missing_or_wrong_locked_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "package.json").write_text(json.dumps({
                "devDependencies": {"typescript": "5.9.3"},
                "dependencies": {"openapi-fetch": "0.17.0"},
            }))
            (root / "pnpm-lock.yaml").write_text(
                "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n"
                "    dependencies:\n      openapi-fetch:\n"
                "        specifier: 0.17.0\n        version: 0.17.0\n"
                "    devDependencies:\n      typescript:\n"
                "        specifier: 5.9.3\n        version: 5.9.3\n"
            )
            self.assertFalse(doctor.frontend_dependencies(root).ok)
            for name, version in [("typescript", "5.9.3"), ("openapi-fetch", "0.17.0")]:
                package = root / "node_modules" / name
                package.mkdir(parents=True)
                (package / "package.json").write_text(json.dumps({"version": version}))
            self.assertTrue(doctor.frontend_dependencies(root).ok)
            (root / "node_modules/typescript/package.json").write_text('{"version":"5.8.0"}')
            result = doctor.frontend_dependencies(root)
            self.assertFalse(result.ok)
            self.assertIn("typescript", result.detail)
            self.assertIn("pnpm install --frozen-lockfile", result.remedy)

    def test_unknown_scope_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "unknown prerequisite scope"):
            doctor.check_scope("everything")


if __name__ == "__main__":
    unittest.main()
