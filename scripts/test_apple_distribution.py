"""Pure signing-policy checks: no Keychain secrets, Apple network, or certificate changes."""
import importlib.util
import json
import pathlib
import unittest

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


if __name__ == '__main__':
    unittest.main()
