#!/usr/bin/env python3
"""Verify released mobile artifacts and prepare store submissions without building code."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import stat
import subprocess
import sys
import urllib.error
import urllib.request
import zipfile

PREDICATE = 'https://github.com/Quavon-dev/tarn-browser/attestations/release/v1'
PACKAGE = 'de.quavon.tarn'
TEAM = 'CFV35FGSHF'


def validate_tag(tag: str) -> None:
    if not re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', tag):
        raise ValueError('store uploads require a stable vX.Y.Z release tag')


def checksum(sums: str, name: str, content: bytes) -> None:
    entries = {}
    for line in sums.splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (?:\./)?([A-Za-z0-9_.+-]+)', line)
        if not match or match[2] in entries or match[2] in ('.', '..'):
            raise ValueError('invalid or duplicate SHA256SUMS entry')
        entries[match[2]] = match[1]
    if entries.get(name) != hashlib.sha256(content).hexdigest():
        raise ValueError(f'checksum missing or mismatched: {name}')


def check_predicate(results: list, commit: str, tag: str) -> None:
    expected = {'commit': commit, 'tag': tag}
    statements = [result.get('verificationResult', {}).get('statement', {}) for result in results]
    if not any(statement.get('predicateType') == PREDICATE and statement.get('predicate') == expected
               for statement in statements):
        raise ValueError('verified attestation does not bind the release tag to its current commit')


def gh(*args: str) -> str:
    return subprocess.check_output(['gh', *args], text=True)


def release_commit(repo: str, tag: str) -> str:
    obj = json.loads(gh('api', f'repos/{repo}/git/ref/tags/{tag}'))['object']
    for _ in range(8):
        if obj['type'] == 'commit' and re.fullmatch(r'[0-9a-f]{40}', obj['sha']):
            return obj['sha']
        if obj['type'] != 'tag' or not re.fullmatch(r'[0-9a-f]{40}', obj['sha']):
            break
        obj = json.loads(gh('api', f'repos/{repo}/git/tags/{obj["sha"]}'))['object']
    raise ValueError('release tag does not resolve to a commit')


def verify(tag: str, platform: str, destination: Path) -> None:
    validate_tag(tag)
    repo = os.environ['GH_REPO']
    if repo != 'Quavon-dev/tarn-browser':
        raise ValueError('store publishing is restricted to Quavon-dev/tarn-browser')
    release = json.loads(gh('api', f'repos/{repo}/releases/tags/{tag}'))
    if release['draft'] or release['prerelease'] or release['tag_name'] != tag:
        raise ValueError('release must be published and stable')
    ios = f'Tarn-{tag}-ios-unsigned.zip'
    android = f'Tarn-{tag}-android-unsigned.aab'
    names = [item['name'] for item in release['assets']]
    if any(names.count(name) != 1 for name in ('SHA256SUMS', ios, android)):
        raise ValueError('release must contain checksums and both mobile artifacts exactly once')
    commit = release_commit(repo, tag)
    destination.mkdir(parents=True, exist_ok=False)
    artifact = ios if platform == 'ios' else android
    for name in ('SHA256SUMS', artifact):
        gh('release', 'download', tag, '--repo', repo, '--pattern', name, '--dir', str(destination))
        verified = json.loads(gh(
            'attestation', 'verify', str(destination / name), '--repo', repo,
            '--deny-self-hosted-runners', '--source-digest', commit,
            '--cert-identity-regex',
            rf'^https://github\.com/{re.escape(repo)}/\.github/workflows/release\.yml@refs/(heads/main|tags/{re.escape(tag)})$',
            '--predicate-type', PREDICATE, '--format', 'json'))
        check_predicate(verified, commit, tag)
    checksum((destination / 'SHA256SUMS').read_text(), artifact, (destination / artifact).read_bytes())
    if release_commit(repo, tag) != commit:
        raise ValueError('release tag moved during verification')
    print(f'Verified {tag} ({commit}): {artifact}')


def extract_ios(archive: Path, destination: Path) -> None:
    with zipfile.ZipFile(archive) as source:
        seen = set()
        entries = source.infolist()
        if len(entries) > 20_000 or sum(entry.file_size for entry in entries) > 1024 ** 3:
            raise ValueError('iOS archive exceeds the extraction budget')
        for entry in entries:
            path = PurePosixPath(entry.filename)
            mode = entry.external_attr >> 16
            if (not path.parts or path.parts[0] != 'Tarn.app' or '..' in path.parts
                    or '\\' in entry.filename or path.is_absolute() or str(path) in seen
                    or stat.S_ISLNK(mode) or (stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR))
                    or any(part.endswith(('.framework', '.appex', '.dylib', '.xpc')) for part in path.parts)):
                raise ValueError('iOS archive contains unsafe paths, links or unsupported nested code')
            seen.add(str(path))
        source.extractall(destination)


def prepare_ios(archive: Path, profile_path: Path, destination: Path, tag: str) -> None:
    validate_tag(tag)
    profile = plistlib.loads(profile_path.read_bytes())
    entitlements = profile['Entitlements']
    if (profile['TeamIdentifier'] != [TEAM]
            or entitlements.get('application-identifier') != f'{TEAM}.{PACKAGE}'
            or entitlements.get('com.apple.developer.team-identifier') != TEAM
            or entitlements.get('get-task-allow', False)
            or profile.get('ProvisionedDevices') or profile.get('ProvisionsAllDevices', False)
            or profile['ExpirationDate'] <= datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)):
        raise ValueError('an unexpired App Store distribution profile for Tarn is required')
    extract_ios(archive, destination)
    app = destination / 'Tarn.app'
    info = plistlib.loads((app / 'Info.plist').read_bytes())
    if (info.get('CFBundleIdentifier') != PACKAGE or info.get('CFBundleExecutable') != 'Tarn'
            or info.get('CFBundleShortVersionString') != tag[1:]
            or not re.fullmatch(r'[0-9]+', str(info.get('CFBundleVersion', '')))):
        raise ValueError('iOS bundle identity, version or build number does not match the release')
    capabilities = info.get('UIRequiredDeviceCapabilities')
    if not isinstance(capabilities, list) or 'arm64' not in capabilities:
        raise ValueError('App Store Connect requires arm64 in UIRequiredDeviceCapabilities')
    (destination.parent / 'entitlements.plist').write_bytes(plistlib.dumps(entitlements))
    fingerprints = [hashlib.sha1(cert).hexdigest().upper() for cert in profile['DeveloperCertificates']]
    (destination.parent / 'profile-certificates.txt').write_text('\n'.join(fingerprints) + '\n')
    (app / 'Tarn').chmod(0o755)


def play_request(method: str, endpoint: str, token: str, body=None, bundle: Path | None = None) -> dict:
    root = 'https://androidpublisher.googleapis.com/'
    headers = {'Authorization': f'Bearer {token}'}
    if bundle:
        data = bundle.read_bytes()
        headers['Content-Type'] = 'application/octet-stream'
    else:
        data = json.dumps(body).encode() if body is not None else None
        headers['Content-Type'] = 'application/json'
    request = urllib.request.Request(root + endpoint, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            content = response.read()
            return json.loads(content) if content else {}
    except urllib.error.HTTPError as error:
        # Store responses may contain account data; report only the status, never the bearer token.
        raise ValueError(f'Play API {method} failed with HTTP {error.code}; inspect Play Console setup') from None


def publish_android(bundle: Path, tag: str) -> None:
    validate_tag(tag)
    token = os.environ['PLAY_ACCESS_TOKEN']
    status = os.environ.get('PLAY_RELEASE_STATUS', 'draft')
    if status not in ('draft', 'completed'):
        raise ValueError('Play internal release status must be draft or completed')
    base = f'androidpublisher/v3/applications/{PACKAGE}/edits'
    edit = play_request('POST', base, token, {})['id']
    if not re.fullmatch(r'[A-Za-z0-9_-]+', edit):
        raise ValueError('unexpected Play edit identifier')
    endpoint = f'{base}/{edit}'
    try:
        uploaded = play_request('POST', f'upload/{endpoint}/bundles?uploadType=media', token, bundle=bundle)
        if uploaded['sha256'].lower() != hashlib.sha256(bundle.read_bytes()).hexdigest():
            raise ValueError('Play uploaded bundle checksum mismatch')
        version = str(uploaded['versionCode'])
        if not re.fullmatch(r'[0-9]+', version):
            raise ValueError('unexpected Play version code')
        play_request('PUT', f'{endpoint}/tracks/internal', token, {
            'track': 'internal', 'releases': [{'name': tag, 'versionCodes': [version], 'status': status}]})
        play_request('POST', f'{endpoint}:validate', token)
        play_request('POST', f'{endpoint}:commit?changesInReviewBehavior=ERROR_IF_IN_REVIEW'
                     '&changesNotSentForReview=true', token)
    except Exception:
        try:
            play_request('DELETE', endpoint, token)
        except (ValueError, urllib.error.URLError):
            print('Could not discard failed Play edit; it expires automatically', file=sys.stderr)
        raise
    print(f'Uploaded {tag}, versionCode {version}, to Play internal ({status})')


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    verify_parser = sub.add_parser('verify')
    verify_parser.add_argument('tag')
    verify_parser.add_argument('platform', choices=('ios', 'android'))
    verify_parser.add_argument('destination', type=Path)
    ios = sub.add_parser('prepare-ios')
    ios.add_argument('archive', type=Path)
    ios.add_argument('profile', type=Path)
    ios.add_argument('destination', type=Path)
    ios.add_argument('tag')
    android = sub.add_parser('publish-android')
    android.add_argument('bundle', type=Path)
    android.add_argument('tag')
    args = parser.parse_args()
    try:
        if args.command == 'verify':
            verify(args.tag, args.platform, args.destination)
        elif args.command == 'prepare-ios':
            prepare_ios(args.archive, args.profile, args.destination, args.tag)
        else:
            publish_android(args.bundle, args.tag)
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
