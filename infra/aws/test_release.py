import hashlib
import importlib.util
import json
import os
import runpy
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('weather_bridge_release', ROOT / 'scripts/release.py')
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


REVISION = '1' * 40
OTHER_REVISION = '2' * 40
TARGET = 'aarch64-unknown-linux-gnu'


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def aarch64_binary(revision=REVISION):
    binary = bytearray(64)
    binary[:6] = b'\x7fELF\x02\x01'
    binary[18:20] = (183).to_bytes(2, 'little')
    return bytes(binary) + revision.encode()


def write_release_zip(path, entries):
    with zipfile.ZipFile(path, 'w') as output:
        for name, content in entries:
            output.writestr(name, content)


class ReleaseVerificationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        write_release_zip(self.directory / 'app.zip', [
            ('weather-bridge', aarch64_binary()),
            ('bootstrap', b'#!/bin/sh\nexec /var/task/weather-bridge serve\n'),
        ])
        write_release_zip(self.directory / 'guard.zip', [
            ('guard.py', b'import json\ndef handler(event, context):\n    return {}\n'),
        ])
        app_content = hashlib.sha256(b'#!/bin/sh\nexec /var/task/weather-bridge serve\n').hexdigest()
        guard_content = hashlib.sha256(
            b'import json\ndef handler(event, context):\n    return {}\n'
        ).hexdigest()
        manifest = {
            'schemaVersion': 1,
            'revision': REVISION,
            'target': TARGET,
            'artifacts': {
                'app.zip': {'sha256': sha256(self.directory / 'app.zip'),
                            'size': (self.directory / 'app.zip').stat().st_size,
                            'contentSha256': app_content},
                'guard.zip': {'sha256': sha256(self.directory / 'guard.zip'),
                              'size': (self.directory / 'guard.zip').stat().st_size,
                              'contentSha256': guard_content},
            },
        }
        (self.directory / 'release-manifest.json').write_text(json.dumps(manifest) + '\n')

    def tearDown(self):
        self.temp.cleanup()

    def test_rejects_missing_manifest(self):
        (self.directory / 'release-manifest.json').unlink()
        with self.assertRaisesRegex(release.ReleaseError, 'manifest is missing'):
            release.verify_release(self.directory, REVISION)

    def test_rejects_wrong_artifact_digest(self):
        (self.directory / 'app.zip').write_bytes(b'tampered')
        with self.assertRaisesRegex(release.ReleaseError, 'app.zip digest does not match'):
            release.verify_release(self.directory, REVISION)

    def test_rejects_wrong_revision(self):
        with self.assertRaisesRegex(release.ReleaseError, 'revision does not match'):
            release.verify_release(self.directory, OTHER_REVISION)

    def test_accepts_matching_release(self):
        manifest = release.verify_release(self.directory, REVISION)
        self.assertEqual(manifest['revision'], REVISION)
        self.assertEqual(manifest['target'], TARGET)

    def test_deploy_rejects_pre_cloudfront_stack_before_terraform(self):
        driver = self.directory / "infra/aws/deploy.py"
        driver.parent.mkdir(parents=True)
        driver.write_bytes((ROOT / "infra/aws/deploy.py").read_bytes())
        calls = []

        def describe(command, **_kwargs):
            calls.append(command)
            self.assertEqual(command[:2], ["aws", "cloudformation"])
            stack = {"StackId": "legacy", "Outputs": [{
                "OutputKey": "Endpoint", "OutputValue": "https://legacy.lambda-url.us-east-1.on.aws/",
            }]}
            return subprocess.CompletedProcess(command, 0, stdout=json.dumps({"Stacks": [stack]}))

        arguments = [str(driver), "--contact", "release-test@example.com",
                     "--release", str(self.directory), "--revision", REVISION]
        with mock.patch.dict(sys.modules, {"release": release}), mock.patch.object(
            sys, "argv", arguments
        ), mock.patch("subprocess.run", side_effect=describe):
            with self.assertRaisesRegex(SystemExit, "CloudFront-based"):
                runpy.run_path(str(driver), run_name="__main__")
        self.assertEqual(len(calls), 2)
        self.assertFalse((self.directory / "target/aws-deployment/origin-verify-secret").exists())

    def test_deploy_rejects_missing_manifest_before_aws_access(self):
        (self.directory / 'release-manifest.json').unlink()
        environment = dict(os.environ)
        environment['PATH'] = ''
        result = subprocess.run([
            sys.executable, str(ROOT / 'infra/aws/deploy.py'),
            '--profile', 'must-not-be-used', '--contact', 'release-test@example.com',
            '--release', str(self.directory), '--revision', REVISION,
        ], text=True, capture_output=True, env=environment)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Release verification failed before AWS access', result.stderr)
        self.assertNotIn('AWS CLI', result.stderr)


class ReleaseArchiveTests(unittest.TestCase):
    def test_package_builds_the_deployed_zip_once_with_revision_and_wrapper(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            compiled = root / 'compiled.zip'
            write_release_zip(compiled, [('bootstrap', aarch64_binary())])
            output = root / 'release'
            with mock.patch.dict(os.environ, {release.REVISION_ENV: REVISION}):
                manifest = release.package_release(compiled, output, REVISION)
            self.assertEqual(manifest, release.verify_release(output, REVISION))
            with zipfile.ZipFile(output / 'app.zip') as deployed:
                self.assertEqual(set(deployed.namelist()), {'weather-bridge', 'bootstrap'})
                self.assertEqual(deployed.read('weather-bridge'), aarch64_binary())
                self.assertEqual(deployed.read('bootstrap'), (ROOT / 'infra/aws/bootstrap').read_bytes())

    def test_extract_requires_the_archive_digest_and_exact_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            source.mkdir()
            for name in release.RELEASE_FILES:
                (source / name).write_bytes(name.encode())
            archive = root / 'release.zip'
            with zipfile.ZipFile(archive, 'w') as output:
                for name in release.RELEASE_FILES:
                    output.write(source / name, name)
            destination = root / 'out'
            release.extract_archive(archive, sha256(archive), destination)
            self.assertEqual(set(path.name for path in destination.iterdir()), set(release.RELEASE_FILES))
            with self.assertRaisesRegex(release.ReleaseError, 'archive digest does not match'):
                release.extract_archive(archive, '0' * 64, root / 'wrong')


class WorkflowArtifactSelectionTests(unittest.TestCase):
    def test_rollback_source_must_be_successful_main_run_of_this_workflow(self):
        run = {
            'id': 42,
            'status': 'completed',
            'conclusion': 'success',
            'event': 'push',
            'path': '.github/workflows/release.yml',
            'head_branch': 'main',
            'head_sha': REVISION,
            'head_repository': {'full_name': 'marknorgren/weather-bridge', 'id': 7},
        }
        artifact = {
            'id': 99,
            'name': 'weather-bridge-release-' + REVISION,
            'expired': False,
            'digest': 'sha256:' + 'a' * 64,
            'workflow_run': {
                'id': 42,
                'head_sha': REVISION,
                'head_repository_id': 7,
            },
        }
        selected = release.select_workflow_artifact(
            run, {'artifacts': [artifact]}, 'marknorgren/weather-bridge', 42
        )
        self.assertEqual(selected['artifact_id'], '99')
        self.assertEqual(selected['revision'], REVISION)
        self.assertEqual(selected['artifact_digest'], 'a' * 64)

        run['conclusion'] = 'failure'
        with self.assertRaisesRegex(release.ReleaseError, 'successful'):
            release.select_workflow_artifact(
                run, {'artifacts': [artifact]}, 'marknorgren/weather-bridge', 42
            )


if __name__ == '__main__':
    unittest.main()
