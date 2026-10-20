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

    def run_deployment(self, resume_results, extra_args=(), previous_email='', stop_result=None, throttles=0,
                       owner_configured='true', stop_results=None):
        driver = self.directory / 'infra/aws/deploy.py'
        driver.parent.mkdir(parents=True, exist_ok=True)
        (driver.parent / 'terraform').mkdir(exist_ok=True)
        driver.write_bytes((ROOT / 'infra/aws/deploy.py').read_bytes())
        calls = []
        actions = []
        outputs = {'endpoint': 'https://example.cloudfront.net/',
                   'function_url': 'https://origin.lambda-url.us-east-1.on.aws/',
                   'distribution_id': 'EEXAMPLE', 'distribution_endpoint': 'https://example.cloudfront.net/'}
        results = iter(resume_results)
        stops = iter(stop_results) if stop_results is not None else None
        remaining_throttles = throttles
        def execute(command, **_kwargs):
            nonlocal remaining_throttles
            calls.append(command)
            value = {}
            if command[0] == 'aws':
                if command[1:3] == ['cloudformation', 'describe-stacks']:
                    stack = command[command.index('--stack-name') + 1]
                    if stack.endswith('artifacts'):
                        value = {'Stacks': [{'StackId': stack, 'Outputs': [{'OutputKey': 'Bucket', 'OutputValue': 'private-bucket'}]}]}
                    else:
                        value = {'Stacks': [{'StackId': stack, 'Outputs': [
                            {'OutputKey': 'Endpoint', 'OutputValue': outputs['endpoint']},
                            {'OutputKey': 'FunctionUrl', 'OutputValue': outputs['function_url']},
                            {'OutputKey': 'OwnerNotificationConfigured', 'OutputValue': owner_configured},
                            {'OutputKey': 'DistributionId', 'OutputValue': 'EEXAMPLE'}],
                            'Parameters': [{'ParameterKey': 'OwnerNotificationEmail', 'ParameterValue': previous_email}]}]}
                elif command[1:3] == ['lambda', 'invoke']:
                    if remaining_throttles:
                        remaining_throttles -= 1
                        return subprocess.CompletedProcess(command, 255, stdout='', stderr=(
                            'An error occurred (TooManyRequestsException) when calling the Invoke operation: busy'))
                    index = command.index('--payload')
                    action = json.loads(command[index + 1])['action']
                    actions.append(action)
                    if action == 'stop':
                        response = next(stops) if stops is not None else (stop_result or {'stopped': True, 'resumeAllowed': True})
                    else:
                        response = next(results)
                    Path(command[index + 2]).write_text(json.dumps(response))
            elif command[0] == 'terraform':
                if command[2] == 'show':
                    value = {'resource_changes': []}
                elif command[2] == 'output':
                    value = outputs[command[-1]]
            else:
                self.fail('unexpected subprocess: ' + str(command))
            return subprocess.CompletedProcess(command, 0, stdout=json.dumps(value))
        args = [str(driver), '--contact', 'test@example.com', '--release', str(self.directory),
                '--revision', REVISION, *extra_args]
        with mock.patch.dict(sys.modules, {'release': release}), mock.patch.object(sys, 'argv', args), mock.patch(
                'subprocess.run', side_effect=execute), mock.patch('time.sleep') as sleep:
            runpy.run_path(str(driver), run_name='__main__')
        settings = (driver.parent / 'terraform/deployment.auto.tfvars').read_text()
        return actions, calls, sleep.call_args_list, settings

    def test_deployment_records_stop_before_updates_and_waits_only_for_alarm_initialization(self):
        actions, calls, sleeps, _ = self.run_deployment([
            {'stopped': True, 'reason': 'control_alarm'}, {'stopped': False}], [])
        self.assertEqual(actions, ['stop', 'stop', 'resume', 'resume'])
        stop_index = next(i for i, call in enumerate(calls) if call[:3] == ['aws', 'lambda', 'invoke'])
        apply_index = next(i for i, call in enumerate(calls) if call[:1] == ['terraform'] and call[2] == 'apply')
        self.assertLess(stop_index, apply_index)
        self.assertEqual(sleeps, [mock.call(30)])

    def test_deployment_keeps_existing_manual_and_legacy_stops(self):
        for stopped in ({'stopped': True, 'reason': 'manual_stop', 'resumeAllowed': False},
                        {'stopped': True, 'reason': 'stop requested'}):
            with self.subTest(stopped=stopped):
                actions, _, _, _ = self.run_deployment([], stop_result=stopped)
                self.assertEqual(actions, ['stop', 'stop'])

    def test_explicit_operator_override_can_resume_an_existing_stop(self):
        actions, calls, _, _ = self.run_deployment([{'stopped': False}], ['--resume-stopped'],
            stop_result={'stopped': True, 'reason': 'manual_stop', 'resumeAllowed': False})
        self.assertEqual(actions[-1], 'resume')
        payloads = [json.loads(c[c.index('--payload') + 1]) for c in calls if c[:3] == ['aws', 'lambda', 'invoke']]
        self.assertNotIn('deployment', payloads[-1])

    def test_throttled_guard_invocation_retries_with_a_timeout_longer_than_lambda(self):
        _, calls, sleeps, _ = self.run_deployment([{'stopped': False}], throttles=1)
        self.assertEqual(sleeps, [mock.call(30)])
        for call in calls:
            if call[:3] == ['aws', 'lambda', 'invoke']:
                self.assertIn('--cli-read-timeout', call)
                self.assertGreater(int(call[call.index('--cli-read-timeout') + 1]), 120)

    def test_masked_owner_address_requires_explicit_configuration(self):
        with self.assertRaisesRegex(SystemExit, 'owner-email'):
            self.run_deployment([], previous_email='****')

    def test_disabled_masked_owner_configuration_can_remain_disabled(self):
        _, _, _, settings = self.run_deployment([{'stopped': False}], previous_email='****',
                                               owner_configured='false')
        self.assertIn('owner_notification_email = ""', settings)

    def test_guard_throttling_has_a_finite_retry_limit(self):
        with self.assertRaisesRegex(RuntimeError, 'TooManyRequestsException'):
            self.run_deployment([], throttles=6)

    def test_legacy_first_stop_cannot_borrow_later_recovery_authority(self):
        actions, _, _, _ = self.run_deployment([], stop_results=[
            {'stopped': True}, {'stopped': True, 'resumeAllowed': True}])
        self.assertEqual(actions, ['stop', 'stop'])

    def test_revoked_deployment_authority_fails_without_alarm_waits(self):
        with self.assertRaisesRegex(RuntimeError, 'ownership'):
            self.run_deployment([{'stopped': True, 'reason': 'control_alarm', 'resumeAllowed': False}])

    def test_owner_email_is_preserved_unless_explicitly_replaced_or_removed(self):
        for args, expected in [([], 'previous@example.com'),
                               (['--owner-email', 'new@example.com'], 'new@example.com'),
                               (['--owner-email='], '')]:
            with self.subTest(args=args):
                _, _, _, settings = self.run_deployment([{'stopped': False}], args, 'previous@example.com')
                self.assertIn('owner_notification_email = ' + json.dumps(expected), settings)

    def test_deployment_does_not_retry_a_budget_stop(self):
        with self.assertRaisesRegex(RuntimeError, 'budget_limit'):
            self.run_deployment([{'stopped': True, 'reason': 'budget_limit'}])

    def test_invalid_owner_email_fails_before_any_external_command(self):
        for email in ('a\nb@example.com', '--bad@example.com', '${file("secret")}@example.com', 'a@b', 'a' * 255):
            with self.subTest(email=email), mock.patch.object(sys, 'argv', [
                    str(ROOT / 'infra/aws/deploy.py'), '--contact', 'test@example.com',
                    '--release', str(self.directory), '--revision', REVISION, '--owner-email=' + email]), mock.patch(
                    'subprocess.run') as run, mock.patch('sys.stderr'):
                with self.assertRaises(SystemExit):
                    runpy.run_path(str(ROOT / 'infra/aws/deploy.py'), run_name='__main__')
                run.assert_not_called()

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
