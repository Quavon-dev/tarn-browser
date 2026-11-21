#!/usr/bin/env python3
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch
import zipfile

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location('mobile_store', Path(__file__).with_name('mobile-store.py'))
store = importlib.util.module_from_spec(spec)
spec.loader.exec_module(store)


class VerificationTests(unittest.TestCase):
    def test_tag_rejects_nightly_and_injection(self):
        for tag in ('nightly', 'v1.2.3-beta', '../v1.2.3', 'v1.2.3\n', '-v1.2.3'):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                store.validate_tag(tag)
        store.validate_tag('v1.2.3')

    def test_checksum_rejects_missing_duplicate_and_paths(self):
        digest = hashlib.sha256(b'build').hexdigest()
        for sums in ('', f'{digest}  ../app.zip\n', f'{digest}  ./app.zip\n{digest}  ./app.zip\n'):
            with self.subTest(sums=sums), self.assertRaises(ValueError):
                store.checksum(sums, 'app.zip', b'build')
        store.checksum(f'{digest}  ./app.zip\n', 'app.zip', b'build')
        with self.assertRaises(ValueError):
            store.checksum(f'{digest}  ./app.zip\n', 'app.zip', b'tampered')

    def test_predicate_binds_commit_and_tag(self):
        def result(commit, tag):
            return [{'verificationResult': {'statement': {'predicateType': store.PREDICATE, 'predicate': {'commit': commit, 'tag': tag}}}}]
        store.check_predicate(result('a' * 40, 'v1.2.3'), 'a' * 40, 'v1.2.3')
        for commit, tag in [('b' * 40, 'v1.2.3'), ('a' * 40, 'v9.9.9')]:
            with self.assertRaises(ValueError):
                store.check_predicate(result(commit, tag), 'a' * 40, 'v1.2.3')
        with self.assertRaises(ValueError):
            store.check_predicate([], 'a' * 40, 'v1.2.3')

    def test_archive_rejects_traversal_symlink_and_nested_code(self):
        for name, mode in [('Tarn.app/../../escape', 0), ('Other.app/Tarn', 0),
                           ('Tarn.app/link', 0o120777), ('Tarn.app/Frameworks/Evil.framework/Evil', 0)]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as folder:
                archive = Path(folder) / 'app.zip'
                with zipfile.ZipFile(archive, 'w') as output:
                    entry = zipfile.ZipInfo(name)
                    entry.external_attr = mode << 16
                    output.writestr(entry, b'untrusted')
                with self.assertRaises(ValueError):
                    store.extract_ios(archive, Path(folder) / 'payload')

    def test_archive_extraction_budget_and_valid_profile(self):
        now = store.datetime.datetime.now(store.datetime.timezone.utc).replace(tzinfo=None)
        profile = {'TeamIdentifier': [store.TEAM], 'ExpirationDate': now + store.datetime.timedelta(days=1),
                   'DeveloperCertificates': [b'synthetic certificate'],
                   'Entitlements': {'application-identifier': f'{store.TEAM}.{store.PACKAGE}',
                                    'com.apple.developer.team-identifier': store.TEAM}}
        info = {'CFBundleIdentifier': store.PACKAGE, 'CFBundleExecutable': 'Tarn',
                'CFBundleShortVersionString': '1.2.3', 'CFBundleVersion': '5'}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / 'app.zip'
            profile_path = root / 'profile.plist'
            profile_path.write_bytes(store.plistlib.dumps(profile))
            with zipfile.ZipFile(archive, 'w') as output:
                output.writestr('Tarn.app/Info.plist', store.plistlib.dumps(info))
                output.writestr('Tarn.app/Tarn', b'synthetic executable')
            destination = root / 'Payload'
            store.prepare_ios(archive, profile_path, destination, 'v1.2.3')
            self.assertEqual((destination / 'Tarn.app/Tarn').stat().st_mode & 0o777, 0o755)
            self.assertEqual(store.plistlib.loads((root / 'entitlements.plist').read_bytes()), profile['Entitlements'])
            oversized = zipfile.ZipInfo('Tarn.app/large')
            oversized.file_size = 1024 ** 3 + 1
            with patch.object(store.zipfile.ZipFile, 'infolist', return_value=[oversized]):
                with self.assertRaises(ValueError):
                    store.extract_ios(archive, root / 'rejected')

    def test_verify_rejects_draft_and_macos_only(self):
        for release in ({'draft': True}, {'draft': False, 'prerelease': False,
                                        'tag_name': 'v1.2.3', 'assets': []}):
            with self.subTest(release=release), patch.dict(store.os.environ, {'GH_REPO': 'Quavon-dev/tarn-browser'}):
                with patch.object(store, 'gh', return_value=store.json.dumps(release)):
                    with self.assertRaises(ValueError):
                        store.verify('v1.2.3', 'ios', Path('never-created'))

    def test_verification_binds_downloads_checksums_and_tag(self):
        tag = 'v1.2.3'
        artifact = f'Tarn-{tag}-ios-unsigned.zip'
        digest = hashlib.sha256(b'build').hexdigest()
        sums = f'{digest}  ./{artifact}\n'.encode()
        release = {'draft': False, 'prerelease': False, 'tag_name': tag,
                   'assets': [{'name': name} for name in ('SHA256SUMS', artifact,
                                                         f'Tarn-{tag}-android-unsigned.aab')]}
        attestation = [{'verificationResult': {'statement': {
            'predicateType': store.PREDICATE, 'predicate': {'commit': 'a' * 40, 'tag': tag}}}}]
        for tampered, moved in ((False, False), (True, False), (False, True)):
            with self.subTest(tampered=tampered, moved=moved), tempfile.TemporaryDirectory() as folder:
                def fake_gh(*args):
                    if args[0] == 'api':
                        return store.json.dumps(release)
                    if args[:2] == ('release', 'download'):
                        name = args[args.index('--pattern') + 1]
                        destination = Path(args[args.index('--dir') + 1])
                        content = sums if name == 'SHA256SUMS' else (b'changed' if tampered else b'build')
                        (destination / name).write_bytes(content)
                        return ''
                    if args[:2] == ('attestation', 'verify'):
                        self.assertEqual(args[args.index('--source-digest') + 1], 'a' * 40)
                        self.assertIn('--deny-self-hosted-runners', args)
                        identity_flags = {'--cert-identity', '--cert-identity-regex',
                                          '--signer-repo', '--signer-workflow'}
                        self.assertEqual(identity_flags.intersection(args), {'--cert-identity-regex'})
                        identity = args[args.index('--cert-identity-regex') + 1]
                        prefix = 'https://github.com/Quavon-dev/tarn-browser/.github/workflows/'
                        for ref in ('main', tag):
                            branch = 'heads/main' if ref == 'main' else f'tags/{tag}'
                            self.assertIsNotNone(store.re.fullmatch(identity, f'{prefix}release.yml@refs/{branch}'))
                        for other in ('release.yml@refs/heads/untrusted', 'evil.yml@refs/heads/main',
                                      'release.yml@refs/tags/v9.9.9'):
                            self.assertIsNone(store.re.fullmatch(identity, prefix + other))

                        return store.json.dumps(attestation)
                    self.fail(f'unexpected gh command: {args}')
                with patch.dict(store.os.environ, {'GH_REPO': 'Quavon-dev/tarn-browser'}):
                    with patch.object(store, 'gh', side_effect=fake_gh):
                        with patch.object(store, 'release_commit', side_effect=['a' * 40, ('b' if moved else 'a') * 40]):
                            if tampered or moved:
                                with self.assertRaises(ValueError):
                                    store.verify(tag, 'ios', Path(folder) / 'verified')
                            else:
                                store.verify(tag, 'ios', Path(folder) / 'verified')

    def test_profile_rejects_wrong_team_debug_and_expired(self):
        now = store.datetime.datetime.now(store.datetime.timezone.utc).replace(tzinfo=None)
        valid = {'TeamIdentifier': [store.TEAM],
                 'ExpirationDate': now + store.datetime.timedelta(days=1),
                 'Entitlements': {'application-identifier': f'{store.TEAM}.{store.PACKAGE}',
                                  'com.apple.developer.team-identifier': store.TEAM}}
        variants = [dict(valid, TeamIdentifier=['OTHER']), dict(valid, ExpirationDate=now),
                    dict(valid, ProvisionedDevices=['device']),
                    dict(valid, Entitlements={**valid['Entitlements'], 'get-task-allow': True})]
        for profile in variants:
            with self.subTest(profile=profile), tempfile.TemporaryDirectory() as folder:
                path = Path(folder) / 'profile.plist'
                path.write_bytes(store.plistlib.dumps(profile))
                with self.assertRaises(ValueError):
                    store.prepare_ios(Path('missing.zip'), path, Path(folder) / 'Payload', 'v1.2.3')

    def test_play_only_internal_draft_and_digest_checked(self):
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder) / 'app.aab'
            bundle.write_bytes(b'bundle')
            digest = hashlib.sha256(b'bundle').hexdigest()
            for status in ('draft', 'completed'):
                responses = [{'id': 'edit'}, {'versionCode': 5, 'sha256': digest}, {}, {}, {}]
                with patch.dict(store.os.environ, {'PLAY_ACCESS_TOKEN': 'synthetic', 'PLAY_RELEASE_STATUS': status}):
                    with patch.object(store, 'play_request', side_effect=responses) as api:
                        store.publish_android(bundle, 'v1.2.3')
                        track = api.call_args_list[2]
                        self.assertTrue(track.args[1].endswith('/tracks/internal'))
                        self.assertEqual(track.args[3]['releases'][0]['status'], status)
                        commit = api.call_args_list[-1]
                        self.assertIn('changesInReviewBehavior=ERROR_IF_IN_REVIEW', commit.args[1])
                        self.assertIn('changesNotSentForReview=true', commit.args[1])
            with patch.dict(store.os.environ, {'PLAY_ACCESS_TOKEN': 'synthetic', 'PLAY_RELEASE_STATUS': 'production'}):
                with patch.object(store, 'play_request') as api, self.assertRaises(ValueError):
                    store.publish_android(bundle, 'v1.2.3')
                api.assert_not_called()
            responses = [{'id': 'edit'}, {'versionCode': 5, 'sha256': 'bad'}, {}]
            with patch.dict(store.os.environ, {'PLAY_ACCESS_TOKEN': 'synthetic', 'PLAY_RELEASE_STATUS': 'draft'}):
                with patch.object(store, 'play_request', side_effect=responses) as api, self.assertRaises(ValueError):
                    store.publish_android(bundle, 'v1.2.3')
                self.assertEqual(api.call_args_list[-1].args[0], 'DELETE')


if __name__ == '__main__':
    unittest.main()
