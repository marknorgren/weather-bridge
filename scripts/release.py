#!/usr/bin/env python3
"""Build and verify immutable Weather Bridge deployment artifacts."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import tempfile
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TARGET = 'aarch64-unknown-linux-gnu'
REVISION_ENV = 'WEATHER_BRIDGE_BUILD_REVISION'
MANIFEST = 'release-manifest.json'
RELEASE_FILES = ('app.zip', 'guard.zip', MANIFEST)
REVISION_RE = re.compile(r'[0-9a-f]{40}\Z')
DIGEST_RE = re.compile(r'[0-9a-f]{64}\Z')


class ReleaseError(ValueError):
    pass


def artifact_path(value, *, root=None):
    """Keep an artifact inside its declared directory, without following artifact links."""
    if '..' in Path(value).parts:
        raise ReleaseError('artifact paths must not contain parent traversal')
    # Operators choose the directory, including absolute downloaded-release paths.
    # Resolve the parent, then check the complete directory prefix before probing
    # the leaf. The separator prevents a sibling such as release-other matching.
    absolute = os.path.abspath(value)
    parent = os.path.realpath(os.path.dirname(absolute))
    path = os.path.normpath(os.path.join(parent, os.path.basename(absolute)))
    boundary = os.path.realpath(root) if root is not None else parent
    if not path.startswith(boundary.rstrip(os.sep) + os.sep):
        raise ReleaseError('artifact path escapes its declared directory')
    if os.path.islink(path):
        raise ReleaseError('artifact paths must not be symlinks')
    return Path(path)


def digest(path):
    path = artifact_path(path)
    value = hashlib.sha256()
    with path.open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def require_revision(value, label='revision'):
    if not isinstance(value, str) or not REVISION_RE.fullmatch(value):
        raise ReleaseError(f'{label} must be exactly 40 lowercase hexadecimal characters')
    return value


def require_digest(value, label='digest'):
    if not isinstance(value, str) or not DIGEST_RE.fullmatch(value):
        raise ReleaseError(f'{label} must be exactly 64 lowercase hexadecimal characters')
    return value


def check_binary(binary, revision):
    require_revision(revision)
    if len(binary) < 64 or binary[:4] != b'\x7fELF':
        raise ReleaseError('weather-bridge is not an ELF executable')
    if binary[4] != 2 or binary[5] != 1:
        raise ReleaseError('weather-bridge is not a little-endian 64-bit ELF executable')
    if struct.unpack_from('<H', binary, 18)[0] != 183:
        raise ReleaseError('weather-bridge is not an AArch64 ELF executable')
    if revision.encode('ascii') not in binary:
        raise ReleaseError('weather-bridge does not contain the requested build revision')


def zip_members(archive, expected):
    names = [info.filename for info in archive.infolist()]
    if len(names) != len(set(names)) or set(names) != set(expected):
        raise ReleaseError('ZIP must contain exactly: ' + ', '.join(expected))
    for info in archive.infolist():
        if info.is_dir() or Path(info.filename).name != info.filename:
            raise ReleaseError('ZIP entries must be regular top-level files')


def inspect_app(path, revision):
    path = artifact_path(path)
    try:
        with zipfile.ZipFile(path) as archive:
            zip_members(archive, ('weather-bridge', 'bootstrap'))
            binary = archive.read('weather-bridge')
            wrapper = archive.read('bootstrap')
    except (OSError, zipfile.BadZipFile, KeyError) as error:
        raise ReleaseError(f'app.zip is invalid: {error}') from error
    check_binary(binary, revision)
    if not wrapper.startswith(b'#!/bin/sh\n') or b'exec /var/task/weather-bridge serve' not in wrapper:
        raise ReleaseError('app.zip does not contain the Weather Bridge Lambda wrapper')
    return hashlib.sha256(wrapper).hexdigest()


def inspect_guard(path):
    path = artifact_path(path)
    try:
        with zipfile.ZipFile(path) as archive:
            zip_members(archive, ('guard.py',))
            guard = archive.read('guard.py')
    except (OSError, zipfile.BadZipFile, KeyError) as error:
        raise ReleaseError(f'guard.zip is invalid: {error}') from error
    if b'def handler(' not in guard:
        raise ReleaseError('guard.zip does not contain the usage guard')
    return hashlib.sha256(guard).hexdigest()


def manifest_artifact(path, content_digest):
    path = artifact_path(path)
    return {'sha256': digest(path), 'size': path.stat().st_size, 'contentSha256': content_digest}


def write_zip(path, entries):
    path = artifact_path(path)
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as archive:
        for name, content, mode in entries:
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.create_system = 3
            entry.compress_type = zipfile.ZIP_DEFLATED
            entry.external_attr = mode << 16
            archive.writestr(entry, content)


def package_release(compiled_zip, output, revision):
    revision = require_revision(revision)
    env_revision = os.environ.get(REVISION_ENV)
    if env_revision != revision:
        raise ReleaseError(f'{REVISION_ENV} must equal the packaged revision')
    compiled_zip = artifact_path(compiled_zip)
    output = artifact_path(output)
    try:
        with zipfile.ZipFile(compiled_zip) as compiled:
            zip_members(compiled, ('bootstrap',))
            binary = compiled.read('bootstrap')
    except (OSError, zipfile.BadZipFile, KeyError) as error:
        raise ReleaseError(f'compiled Lambda ZIP is invalid: {error}') from error
    check_binary(binary, revision)
    if output.exists() and any(output.iterdir()):
        raise ReleaseError(f'release output is not empty: {output}')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.weather-bridge-release-', dir=output.parent) as temporary:
        staging = artifact_path(temporary, root=output.parent)
        app = artifact_path(staging / 'app.zip', root=staging)
        guard = artifact_path(staging / 'guard.zip', root=staging)
        write_zip(app, [
            ('weather-bridge', binary, 0o755),
            ('bootstrap', (ROOT / 'infra/aws/bootstrap').read_bytes(), 0o755),
        ])
        write_zip(guard, [
            ('guard.py', (ROOT / 'infra/aws/guard.py').read_bytes(), 0o644),
        ])
        manifest = {
            'schemaVersion': 1,
            'revision': revision,
            'target': TARGET,
            'artifacts': {
                'app.zip': manifest_artifact(app, inspect_app(app, revision)),
                'guard.zip': manifest_artifact(guard, inspect_guard(guard)),
            },
        }
        artifact_path(staging / MANIFEST, root=staging).write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + '\n')
        verify_release(staging, revision)
        output.mkdir(parents=True, exist_ok=True)
        for name in RELEASE_FILES:
            os.replace(artifact_path(staging / name, root=staging),
                       artifact_path(output / name, root=output))
    return manifest


def load_manifest(directory):
    directory = artifact_path(directory)
    path = artifact_path(directory / MANIFEST, root=directory)
    if not path.is_file():
        raise ReleaseError(f'release manifest is missing: {path}')
    if path.stat().st_size > 16 * 1024:
        raise ReleaseError('release manifest is too large')
    try:
        manifest = json.loads(path.read_text())
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseError(f'release manifest is invalid: {error}') from error
    return manifest


def verify_release(directory, expected_revision):
    directory = artifact_path(directory)
    expected_revision = require_revision(expected_revision, 'expected revision')
    manifest = load_manifest(directory)
    if not isinstance(manifest, dict) or manifest.get('schemaVersion') != 1:
        raise ReleaseError('release manifest schema is unsupported')
    revision = require_revision(manifest.get('revision'), 'manifest revision')
    if revision != expected_revision:
        raise ReleaseError('release manifest revision does not match the expected revision')
    if manifest.get('target') != TARGET:
        raise ReleaseError(f'release manifest target must be {TARGET}')
    artifacts = manifest.get('artifacts')
    if not isinstance(artifacts, dict) or set(artifacts) != {'app.zip', 'guard.zip'}:
        raise ReleaseError('release manifest must describe app.zip and guard.zip')
    content_checks = {'app.zip': inspect_app, 'guard.zip': inspect_guard}
    for name in ('app.zip', 'guard.zip'):
        path = artifact_path(directory / name, root=directory)
        record = artifacts[name]
        if not path.is_file() or not isinstance(record, dict):
            raise ReleaseError(f'{name} is missing from the release')
        expected_digest = require_digest(record.get('sha256'), f'{name} digest')
        if digest(path) != expected_digest:
            raise ReleaseError(f'{name} digest does not match the release manifest')
        if record.get('size') != path.stat().st_size:
            raise ReleaseError(f'{name} size does not match the release manifest')
        expected_content = require_digest(record.get('contentSha256'), f'{name} content digest')
        found_content = content_checks[name](path, revision) if name == 'app.zip' else content_checks[name](path)
        if found_content != expected_content:
            raise ReleaseError(f'{name} content does not match the release manifest')
    return manifest


def extract_archive(archive, expected_digest, destination):
    archive = artifact_path(archive)
    destination = artifact_path(destination)
    expected_digest = require_digest(expected_digest, 'archive digest')
    if digest(archive) != expected_digest:
        raise ReleaseError('release archive digest does not match GitHub')
    if destination.exists() and any(destination.iterdir()):
        raise ReleaseError(f'extraction directory is not empty: {destination}')
    destination.mkdir(parents=True, exist_ok=True)
    try:
        with zipfile.ZipFile(archive) as source:
            zip_members(source, RELEASE_FILES)
            for name in RELEASE_FILES:
                artifact_path(destination / name, root=destination).write_bytes(source.read(name))
    except (OSError, zipfile.BadZipFile, KeyError) as error:
        raise ReleaseError(f'release archive is invalid: {error}') from error


def select_workflow_artifact(run, artifacts_response, repository, run_id):
    require_revision(run.get('head_sha'), 'workflow head revision')
    source = run.get('head_repository') or {}
    valid = (
        run.get('id') == run_id
        and run.get('status') == 'completed'
        and run.get('conclusion') == 'success'
        and run.get('event') in {'push', 'workflow_dispatch'}
        and run.get('path') == '.github/workflows/release.yml'
        and run.get('head_branch') == 'main'
        and source.get('full_name') == repository
    )
    if not valid:
        raise ReleaseError('rollback source must be a successful main-branch run of this release workflow and repository')
    name = 'weather-bridge-release-' + run['head_sha']
    matches = [item for item in artifacts_response.get('artifacts', [])
               if item.get('name') == name and not item.get('expired')]
    if len(matches) != 1:
        raise ReleaseError(f'rollback source must have exactly one unexpired {name} artifact')
    artifact = matches[0]
    source_run = artifact.get('workflow_run') or {}
    if (source_run.get('id') != run_id or source_run.get('head_sha') != run['head_sha']
            or source_run.get('head_repository_id') != source.get('id')):
        raise ReleaseError('rollback artifact provenance does not match the selected workflow run')
    api_digest = artifact.get('digest', '')
    if not api_digest.startswith('sha256:'):
        raise ReleaseError('rollback artifact has no GitHub SHA-256 digest')
    artifact_digest = require_digest(api_digest.removeprefix('sha256:'), 'GitHub artifact digest')
    return {'artifact_id': str(artifact['id']), 'artifact_digest': artifact_digest,
            'revision': run['head_sha']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest='command', required=True)
    package = subparsers.add_parser('package')
    package.add_argument('--compiled-zip', required=True, type=artifact_path)
    package.add_argument('--output', required=True, type=artifact_path)
    package.add_argument('--revision', required=True)
    verify = subparsers.add_parser('verify')
    verify.add_argument('--release', required=True, type=artifact_path)
    verify.add_argument('--revision', required=True)
    extract = subparsers.add_parser('extract-artifact')
    extract.add_argument('--archive', required=True, type=artifact_path)
    extract.add_argument('--digest', required=True)
    extract.add_argument('--output', required=True, type=artifact_path)
    select = subparsers.add_parser('select-artifact')
    select.add_argument('--run', required=True, type=artifact_path)
    select.add_argument('--artifacts', required=True, type=artifact_path)
    select.add_argument('--repository', required=True)
    select.add_argument('--run-id', required=True, type=int)
    select.add_argument('--github-output', type=artifact_path)
    args = parser.parse_args()
    try:
        if args.command == 'package':
            manifest = package_release(args.compiled_zip, args.output, args.revision)
            print(json.dumps(manifest, indent=2, sort_keys=True))
        elif args.command == 'verify':
            manifest = verify_release(args.release, args.revision)
            print(json.dumps(manifest, indent=2, sort_keys=True))
        elif args.command == 'extract-artifact':
            extract_archive(args.archive, args.digest, args.output)
        else:
            selected = select_workflow_artifact(
                json.loads(args.run.read_text()), json.loads(args.artifacts.read_text()),
                args.repository, args.run_id,
            )
            lines = ''.join(f'{key}={value}\n' for key, value in selected.items())
            if args.github_output:
                with args.github_output.open('a') as output:
                    output.write(lines)
            else:
                print(lines, end='')
    except ReleaseError as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
