import hashlib
from contextlib import redirect_stderr
from io import StringIO
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


class ArtifactPathTests(unittest.TestCase):
    def test_rejects_sibling_with_a_matching_directory_prefix(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'release'
            root.mkdir()
            with self.assertRaisesRegex(release.ReleaseError, 'escapes'):
                release.artifact_path(Path(tmp) / 'release-other/app.zip', root=root)

    def test_rejects_a_child_parent_link_outside_the_declared_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'release'
            root.mkdir()
            outside = Path(tmp) / 'outside'
            outside.mkdir()
            (root / 'nested').symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(release.ReleaseError, 'escapes'):
                release.artifact_path(root / 'nested/app.zip', root=root)
            self.assertEqual(list(outside.iterdir()), [])


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

    def test_rejects_artifact_symlink_even_when_its_digest_matches(self):
        original = self.directory / 'app.zip'
        outside = self.directory / 'outside-app.zip'
        original.rename(outside)
        original.symlink_to(outside)
        with self.assertRaisesRegex(release.ReleaseError, 'symlink'):
            release.verify_release(self.directory, REVISION)

    def test_rejects_release_directory_symlink(self):
        alias = self.directory / 'alias'
        alias.symlink_to(self.directory, target_is_directory=True)
        with self.assertRaisesRegex(release.ReleaseError, 'symlink'):
            release.verify_release(alias, REVISION)

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

    def test_deploy_rejects_preexisting_staging_symlinks_before_copying(self):
        for linked in ('directory', *release.RELEASE_FILES):
            with self.subTest(linked=linked), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                driver = root / 'infra/aws/deploy.py'
                driver.parent.mkdir(parents=True)
                driver.write_bytes((ROOT / 'infra/aws/deploy.py').read_bytes())
                outside = root / 'outside'
                outside.mkdir()
                for name in release.RELEASE_FILES:
                    (outside / name).write_bytes(b'keep this file')
                staging = root / 'target/aws-deployment'
                staging.parent.mkdir()
                if linked == 'directory':
                    staging.symlink_to(outside, target_is_directory=True)
                else:
                    staging.mkdir()
                    (staging / linked).symlink_to(outside / linked)
                arguments = [str(driver), '--contact', 'release-test@example.com',
                             '--release', str(self.directory), '--revision', REVISION]
                with mock.patch.dict(sys.modules, {'release': release}), mock.patch.object(
                    sys, 'argv', arguments
                ), mock.patch('subprocess.run') as run:
                    with self.assertRaisesRegex(SystemExit, 'Staged release verification failed before AWS access'):
                        runpy.run_path(str(driver), run_name='__main__')
                    run.assert_not_called()
                for name in release.RELEASE_FILES:
                    self.assertEqual((outside / name).read_bytes(), b'keep this file')

    def test_deploy_rejects_option_and_control_injection_before_aws_access(self):
        driver = self.directory / 'infra/aws/deploy.py'
        driver.parent.mkdir(parents=True)
        driver.write_bytes((ROOT / 'infra/aws/deploy.py').read_bytes())
        for option, value in (
            ('profile', '--endpoint-url=http://127.0.0.1:1'),
            ('profile', 'default\n--debug'),
            ('profile', 'default\x1b[2J'),
            ('profile', 'default\x00'),
            ('profile', ' default'),
            ('profile', 'a' * 129),
            ('region', '--debug'),
            ('region', 'us-east-1 --debug'),
        ):
            arguments = [str(driver), '--contact', 'release-test@example.com',
                         '--release', str(self.directory), '--revision', REVISION,
                         '--' + option + '=' + value]
            diagnostic = StringIO()
            with self.subTest(option=option, value=value), redirect_stderr(diagnostic), mock.patch.dict(
                sys.modules, {'release': release}
            ), mock.patch.object(sys, 'argv', arguments), mock.patch('subprocess.run') as run:
                with self.assertRaises(SystemExit) as failure:
                    runpy.run_path(str(driver), run_name='__main__')
                self.assertEqual(failure.exception.code, 2)
                self.assertIn('Invalid AWS ' + option, diagnostic.getvalue())
                run.assert_not_called()

    def test_deploy_preserves_profile_and_partition_region_values_as_single_options(self):
        driver = self.directory / 'infra/aws/deploy.py'
        driver.parent.mkdir(parents=True)
        driver.write_bytes((ROOT / 'infra/aws/deploy.py').read_bytes())
        for profile, region in ((None, 'us-east-1'), ('work sso', 'us-gov-west-1'),
                                ('team/production', 'cn-north-1'), ('work', 'eusc-de-east-1'),
                                ('a' * 128, 'us-east-1'), ('work:/team_@prod.1-SSO', 'us-east-1')):
            calls = []

            def describe(command, **_kwargs):
                calls.append(command)
                self.assertEqual(command[:2], ['aws', 'cloudformation'])
                self.assertIn('--region=' + region, command)
                if profile is None:
                    self.assertFalse(any(part.startswith('--profile') for part in command))
                else:
                    self.assertIn('--profile=' + profile, command)
                stack = {'Outputs': [{'OutputKey': 'Endpoint', 'OutputValue': 'https://legacy.example/'}]}
                return subprocess.CompletedProcess(command, 0, stdout=json.dumps({'Stacks': [stack]}))

            arguments = [str(driver), '--contact', 'release-test@example.com',
                         '--release', str(self.directory), '--revision', REVISION, '--region', region]
            if profile is not None:
                arguments.extend(('--profile', profile))
            with self.subTest(profile=profile, region=region), mock.patch.dict(
                sys.modules, {'release': release}
            ), mock.patch.object(sys, 'argv', arguments), mock.patch('subprocess.run', side_effect=describe):
                with self.assertRaisesRegex(SystemExit, 'CloudFront-based'):
                    runpy.run_path(str(driver), run_name='__main__')
            self.assertEqual(len(calls), 2)


class ReleaseArchiveTests(unittest.TestCase):
    def test_package_rejects_traversal_without_writing_an_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            compiled = root / 'compiled.zip'
            write_release_zip(compiled, [('bootstrap', aarch64_binary())])
            (root / 'unused').mkdir()
            with mock.patch.dict(os.environ, {release.REVISION_ENV: REVISION}):
                with self.assertRaisesRegex(release.ReleaseError, 'traversal'):
                    release.package_release(compiled, root / 'unused/../release', REVISION)
            self.assertFalse((root / 'release').exists())

    def test_package_rejects_compiled_zip_symlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            compiled = root / 'compiled.zip'
            write_release_zip(compiled, [('bootstrap', aarch64_binary())])
            alias = root / 'alias.zip'
            alias.symlink_to(compiled)
            with mock.patch.dict(os.environ, {release.REVISION_ENV: REVISION}):
                with self.assertRaisesRegex(release.ReleaseError, 'symlink'):
                    release.package_release(alias, root / 'release', REVISION)
            self.assertFalse((root / 'release').exists())

    def test_extract_rejects_symlink_destination_without_writing_through_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'release.zip'
            write_release_zip(archive, [(name, name.encode()) for name in release.RELEASE_FILES])
            outside = root / 'outside'
            outside.mkdir()
            alias = root / 'alias'
            alias.symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(release.ReleaseError, 'symlink'):
                release.extract_archive(archive, sha256(archive), alias)
            self.assertEqual(list(outside.iterdir()), [])

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

    def test_extract_rejects_traversal_absolute_and_duplicate_archive_members(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            outside = root / 'outside'
            outside.write_bytes(b'keep this file')
            for app_name in ('../outside', str(outside), 'nested/app.zip'):
                with self.subTest(app_name=app_name):
                    archive = root / 'release.zip'
                    write_release_zip(archive, [(app_name, b'changed'),
                                               ('guard.zip', b'guard'),
                                               (release.MANIFEST, b'{}')])
                    with self.assertRaisesRegex(release.ReleaseError, 'ZIP must contain exactly'):
                        release.extract_archive(archive, sha256(archive), root / 'out')
                    self.assertEqual(outside.read_bytes(), b'keep this file')
                    self.assertEqual(list((root / 'out').iterdir()), [])

            archive = root / 'duplicate.zip'
            with self.assertWarns(UserWarning):
                write_release_zip(archive, [('app.zip', b'one'), ('app.zip', b'two'),
                                           ('guard.zip', b'guard'), (release.MANIFEST, b'{}')])
            with self.assertRaisesRegex(release.ReleaseError, 'ZIP must contain exactly'):
                release.extract_archive(archive, sha256(archive), root / 'duplicate-out')


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
