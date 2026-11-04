"""Pure signing-policy checks: no Keychain secrets, Apple network, or certificate changes."""
import importlib.util
import json
import pathlib
import plistlib
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


identity = load('signing_identity', 'apple-signing-identity.py')
notarize = load('notarization', 'notarize-macos.py')


class SigningPolicyTests(unittest.TestCase):
    def test_selects_only_developer_id_application_for_the_expected_team(self):
        expected, other = 'ABCDEFGHIJ', 'KLMNOPQRST'
        data = '\n'.join([
            f'1) {"A" * 40} "Developer ID Application: Example ({expected})"',
            f'2) {"B" * 40} "Developer ID Application: Example ({other})"',
            f'3) {"C" * 40} "Apple Development: Example ({expected})"',
        ])
        self.assertEqual(identity.candidates(data, expected), ['A' * 40])

    def test_distribution_rejects_wrong_team_development_adhoc_or_missing_timestamp(self):
        text = '\n'.join(['Identifier=org.example.app', 'TeamIdentifier=ABCDEFGHIJ',
                           'Authority=Developer ID Application: Example (ABCDEFGHIJ)',
                           'CodeDirectory flags=0x10000(runtime)', 'Timestamp=synthetic'])
        notarize.verify_metadata(text, 'org.example.app', 'ABCDEFGHIJ')
        for malformed in [text.replace('TeamIdentifier=ABCDEFGHIJ', 'TeamIdentifier=KLMNOPQRST'),
                          text.replace('Identifier=org.example.app', 'Identifier=org.example.other'),
                          text.replace('Developer ID Application:', 'Apple Development:'),
                          text + '\nSignature=adhoc', text.replace('Timestamp=synthetic', ''),
                          text.replace('CodeDirectory flags=0x10000(runtime)', 'CodeDirectory flags=0x0')]:
            with self.assertRaises(ValueError):
                notarize.verify_metadata(malformed, 'org.example.app', 'ABCDEFGHIJ')

    def test_namespaces_are_consistent(self):
        settings = json.loads((ROOT / 'config/distribution.json').read_text())
        defaults = json.loads((ROOT / 'config/default.json').read_text())
        tauri = json.loads((ROOT / 'src-tauri/tauri.conf.json').read_text())
        self.assertEqual(settings['runtime_identifier'], 'org.9h41.teams')
        self.assertEqual(settings['configurator_identifier'], 'org.9h41.teams.configurator')
        self.assertEqual(defaults['identifier'], settings['runtime_identifier'])
        self.assertEqual(tauri['identifier'], settings['runtime_identifier'])


class ConfiguratorReleaseTests(unittest.TestCase):
    def app(self, root):
        app = root / 'Teams Configurator.app'
        (app / 'Contents').mkdir(parents=True)
        (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleShortVersionString': '1.0.0'}))
        return app

    @staticmethod
    def fake_ditto(command, **_kwargs):
        with zipfile.ZipFile(command[-1], 'w') as archive:
            archive.writestr('Teams Configurator.app/Contents/Info.plist', b'synthetic bundle')

    def test_only_configurator_archive_and_matching_manifest_replace_old_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            app = self.app(root)
            output = root / 'distribution'
            output.mkdir()
            (output / 'Microsoft Teams.zip').write_bytes(b'obsolete runtime')
            (output / 'release.json').write_text('{}')
            with mock.patch.object(notarize.subprocess, 'run', side_effect=self.fake_ditto):
                archive = notarize.publish_release(app, {'team_id': 'ABCDEFGHIJ'}, 'synthetic-id', 'a' * 40, 'arm64', output)
            self.assertEqual(archive.name, 'Teams Configurator.zip')
            self.assertEqual({p.name for p in output.iterdir()}, {'Teams Configurator.zip', 'SHA256SUMS', 'release.json'})
            manifest = json.loads((output / 'release.json').read_text())
            self.assertEqual(manifest['distribution'], 'configurator-only')
            self.assertEqual(manifest['commit'], 'a' * 40)
            self.assertEqual(len(manifest['archives']), 1)
            self.assertIn(manifest['archives'][0]['sha256'], (output / 'SHA256SUMS').read_text())
            self.assertEqual(manifest['archives'][0]['size_bytes'], archive.stat().st_size)

    def test_failed_zip_creation_preserves_the_previous_release(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            app = self.app(root)
            output = root / 'distribution'
            output.mkdir()
            (output / 'previous.zip').write_bytes(b'previous')
            with mock.patch.object(notarize.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['synthetic'])):
                with self.assertRaises(subprocess.CalledProcessError):
                    notarize.publish_release(app, {'team_id': 'ABCDEFGHIJ'}, 'synthetic-id', 'a' * 40, 'arm64', output)
            self.assertEqual((output / 'previous.zip').read_bytes(), b'previous')
            self.assertFalse(list(root.glob('.configurator-release-*')))

    def test_failed_publication_rolls_the_previous_directory_back(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            app = self.app(root)
            output = root / 'distribution'
            output.mkdir()
            (output / 'previous.zip').write_bytes(b'previous')
            rename = pathlib.Path.rename
            def fail_stage(path, target):
                if path.name.startswith('.configurator-release-'):
                    raise OSError('synthetic publication failure')
                return rename(path, target)
            with mock.patch.object(notarize.subprocess, 'run', side_effect=self.fake_ditto), mock.patch.object(pathlib.Path, 'rename', fail_stage):
                with self.assertRaises(OSError):
                    notarize.publish_release(app, {'team_id': 'ABCDEFGHIJ'}, 'synthetic-id', 'a' * 40, 'arm64', output)
            self.assertEqual((output / 'previous.zip').read_bytes(), b'previous')
            self.assertFalse(list(root.glob('.previous-distribution-*')))

    def test_refuses_standalone_runtime_or_symlinked_distribution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            with self.assertRaises(ValueError):
                notarize.publish_release(root / 'Microsoft Teams.app', {}, '', '', '', root / 'distribution')
            app = self.app(root)
            actual = root / 'actual'
            actual.mkdir()
            output = root / 'distribution'
            output.symlink_to(actual)
            with self.assertRaises(ValueError):
                notarize.publish_release(app, {}, '', '', '', output)
            self.assertTrue(output.is_symlink())


if __name__ == '__main__':
    unittest.main()
