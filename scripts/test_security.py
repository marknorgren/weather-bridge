"""Exercise contributor-tool security boundaries without credentials or network access."""

import os
import json
from pathlib import Path
import select
import subprocess
import tempfile
import unittest
from urllib.parse import urlsplit
from urllib.request import ProxyHandler, build_opener

from scripts.dev import fixture_binary


ROOT = Path(__file__).resolve().parents[1]


class BuildDirectiveTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.build_script = Path(cls.temporary.name) / 'build-script'
        subprocess.run(['rustc', str(ROOT / 'build.rs'), '-o', str(cls.build_script)], check=True,
                       capture_output=True, text=True, timeout=60)

    def run_script(self, revision=None):
        environment = dict(os.environ)
        environment.pop('WEATHER_BRIDGE_BUILD_REVISION', None)
        if revision is not None:
            environment['WEATHER_BRIDGE_BUILD_REVISION'] = revision
        return subprocess.run([str(self.build_script)], env=environment, capture_output=True,
                              text=True, timeout=5)

    def test_missing_revision_emits_only_the_expected_directives(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), [
            'cargo:rerun-if-env-changed=WEATHER_BRIDGE_BUILD_REVISION',
            'cargo:rustc-env=WEATHER_BRIDGE_BUILD_REVISION_EMBEDDED=unknown',
        ])

    def test_valid_revision_round_trips(self):
        revision = '0123456789abcdef' * 2 + '01234567'
        result = self.run_script(revision)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines()[-1],
                         'cargo:rustc-env=WEATHER_BRIDGE_BUILD_REVISION_EMBEDDED=' + revision)

    def test_revision_cannot_inject_a_cargo_directive_or_terminal_control(self):
        for revision in (
            'a' * 40 + '\ncargo:rustc-cfg=forged',
            'a' * 39 + '\r', 'a' * 39 + '\n', 'a' * 39 + '\x1b',
            'A' * 40, 'g' * 40, 'a' * 39, 'a' * 41,
        ):
            with self.subTest(revision=repr(revision)):
                result = self.run_script(revision)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout.splitlines(), [
                    'cargo:rerun-if-env-changed=WEATHER_BRIDGE_BUILD_REVISION',
                ])
                self.assertNotIn('cargo:rustc-cfg=forged', result.stderr)


class FixtureLogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cargo', 'build', '--locked', '--example', 'dev-fixture'], cwd=ROOT,
                       check=True, capture_output=True, text=True, timeout=120)
        cls.fixture = fixture_binary()

    def test_readiness_reports_the_bound_port_for_addresses_and_hostnames(self):
        for bind in ('127.0.0.1:0', 'localhost:0', '[::1]:0'):
            with self.subTest(bind=bind):
                environment = dict(os.environ, WEATHER_BRIDGE_DEV_BIND=bind)
                process = subprocess.Popen([str(self.fixture), 'healthy'], cwd=ROOT,
                                           env=environment, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, text=True)
                try:
                    readable, _, _ = select.select([process.stderr], [], [], 5)
                    self.assertTrue(readable, 'fixture did not report readiness')
                    line = process.stderr.readline().rstrip('\n')
                    prefix = 'SYNTHETIC OFFLINE FIXTURE [healthy] ready at '
                    self.assertTrue(line.startswith(prefix), line)
                    url = line.removeprefix(prefix)
                    address = urlsplit(url)
                    self.assertIn(address.hostname, ('127.0.0.1', '::1'))
                    self.assertGreater(address.port, 0)
                    # Bypass workstation proxies: this check only contacts our child server.
                    with build_opener(ProxyHandler({})).open(url + '/healthz', timeout=5) as response:
                        self.assertEqual(json.load(response), {'status': 'ok'})
                finally:
                    process.terminate()
                    try:
                        stdout, _ = process.communicate(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        stdout, _ = process.communicate(timeout=5)
                self.assertIsNotNone(process.returncode)
                self.assertEqual(stdout, '')

    def test_invalid_inputs_cannot_forge_error_lines_or_terminal_controls(self):
        for scenario, bind in (
            ('unknown\nFORGED\r\x1b[2J', '127.0.0.1:0'),
            ('healthy', '127.0.0.1:0\nFORGED\r\x1b[2J'),
        ):
            with self.subTest(scenario=repr(scenario), bind=repr(bind)):
                environment = dict(os.environ, WEATHER_BRIDGE_DEV_BIND=bind)
                result = subprocess.run([str(self.fixture), scenario], cwd=ROOT, env=environment,
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, '')
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
                self.assertTrue(result.stderr.startswith('SYNTHETIC OFFLINE FIXTURE error: '))
                self.assertNotIn('\r', result.stderr)
                self.assertNotIn('\x1b', result.stderr)
                self.assertFalse(any(line.startswith('FORGED') for line in result.stderr.splitlines()))


if __name__ == '__main__':
    unittest.main()
