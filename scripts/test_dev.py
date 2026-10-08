#!/usr/bin/env python3
"""Focused tests for the development rebuild loop."""

from io import StringIO
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

from scripts import dev


class DevelopmentLoopTests(unittest.TestCase):
    def test_snapshot_tracks_sources_but_excludes_generated_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in (
                "src/main.rs",
                "build.rs",
                "tsconfig.json",
                "data/cities.json",
                "web/vendor/scalar-1.72.1.js",
                "web/weather.ts",
                "web/developer.js",
                "frontend/build.mjs",
                "openapi.json",
                "frontend/schema.d.ts",
                "web/weather.js",
                "target/debug/example",
                "node_modules/tool/index.js",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative)

            snapshot = dev.source_snapshot(root)

        self.assertIn("src/main.rs", snapshot)
        self.assertIn("build.rs", snapshot)
        self.assertIn("tsconfig.json", snapshot)
        self.assertIn("data/cities.json", snapshot)
        self.assertNotIn("web/vendor/scalar-1.72.1.js", snapshot)
        self.assertIn("web/weather.ts", snapshot)
        self.assertIn("web/developer.js", snapshot)
        self.assertIn("frontend/build.mjs", snapshot)
        self.assertNotIn("openapi.json", snapshot)
        self.assertNotIn("frontend/schema.d.ts", snapshot)
        self.assertNotIn("web/weather.js", snapshot)
        self.assertFalse(any(path.startswith("target/") for path in snapshot))
        self.assertFalse(any(path.startswith("node_modules/") for path in snapshot))

    def test_failed_build_is_visible_and_next_rebuild_recovers(self):
        output = StringIO()
        loop = dev.DevelopmentLoop("healthy", output=output)
        statuses = [1, 0, 0, 0]
        with mock.patch.object(loop, "run_build", side_effect=statuses), mock.patch.object(
            loop, "start_server"
        ) as start:
            self.assertFalse(loop.rebuild())
            self.assertTrue(loop.rebuild())

        self.assertIn("build failed; watching for changes to retry", output.getvalue())
        self.assertIn("build recovered", output.getvalue())
        start.assert_called_once()

    def test_interruption_stops_an_active_build_process_group(self):
        stopping = threading.Event()
        loop = dev.DevelopmentLoop("healthy", output=StringIO(), stopping=stopping)
        timer = threading.Timer(0.15, stopping.set)
        timer.start()
        started = time.monotonic()
        try:
            status = loop.run_build([sys.executable, "-c", "import time; time.sleep(60)"])
        finally:
            timer.cancel()

        self.assertIsNone(status)
        self.assertLess(time.monotonic() - started, 2)
        self.assertIsNone(loop.build_process)

    def test_cargo_metadata_selects_custom_target_directory(self):
        metadata = subprocess.CompletedProcess(
            ["cargo"], 0, stdout='{"target_directory":"/tmp/custom-target"}', stderr=""
        )
        with mock.patch.object(dev.subprocess, "run", return_value=metadata):
            binary = dev.fixture_binary()
        self.assertEqual(binary, Path("/tmp/custom-target/debug/examples/dev-fixture"))


if __name__ == "__main__":
    unittest.main()
