#!/usr/bin/env python3
"""Build, Developer-ID sign, notarize and staple the two public macOS bundles.
Authentication is referenced only through a notarytool Keychain profile.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]


def verify_metadata(text, expected_id, team):
    lines = text.splitlines()
    if f'Identifier={expected_id}' not in lines or f'TeamIdentifier={team}' not in lines:
        raise ValueError('Bundle signing identity does not match the configured namespace/team')
    if not any(line.startswith('Authority=Developer ID Application:') for line in lines):
        raise ValueError('Distribution requires a Developer ID Application certificate')
    if 'Signature=adhoc' in lines or not any('runtime' in line for line in lines if line.startswith('CodeDirectory')):
        raise ValueError('Distribution requires Developer ID signing with hardened runtime')
    if not any(line.startswith('Timestamp=') for line in lines):
        raise ValueError('Distribution signature requires a secure timestamp')


def verify_bundle(app, expected_id, team):
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    result = subprocess.run(['/usr/bin/codesign', '-dv', '--verbose=4', str(app)], capture_output=True, text=True, check=True)
    verify_metadata(result.stdout + result.stderr, expected_id, team)


def notarize(app, profile, work):
    archive = work / (app.stem + '-submission.zip')
    subprocess.run(['/usr/bin/ditto', '-c', '-k', '--keepParent', str(app), str(archive)], check=True)
    result = subprocess.run(['xcrun', 'notarytool', 'submit', str(archive), '--keychain-profile', profile,
                             '--wait', '--timeout', '20m', '--output-format', 'json'], capture_output=True, text=True, check=True)
    receipt = json.loads(result.stdout)
    (work / (app.stem + '-receipt.json')).write_text(json.dumps(receipt, indent=2) + '\n')
    request_id = receipt.get('id', '')
    try:
        valid_id = str(uuid.UUID(request_id)) == request_id.lower()
    except (ValueError, AttributeError):
        valid_id = False
    if not valid_id:
        raise ValueError('Notary service did not return a valid submission ID')
    if receipt.get('status') != 'Accepted':
        log = subprocess.run(['xcrun', 'notarytool', 'log', request_id, '--keychain-profile', profile], capture_output=True, text=True)
        (work / (app.stem + '-notary-log.json')).write_text(log.stdout)
        raise ValueError(f'Notarization was not accepted. Submission {request_id}; report directory: {work}')
    subprocess.run(['xcrun', 'stapler', 'staple', str(app)], check=True)
    subprocess.run(['xcrun', 'stapler', 'validate', str(app)], check=True)
    subprocess.run(['/usr/sbin/spctl', '--assess', '--type', 'execute', '--verbose=2', str(app)], check=True)
    print(f'Notarized and stapled: {app.name} (submission {request_id})')
    return request_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', required=True, help='Existing notarytool Keychain profile; never a password')
    parser.add_argument('--identity', help='Optional Developer ID Application SHA-1 fingerprint')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('macOS is required')
    if not args.profile or args.profile.startswith('-') or len(args.profile) > 128 or any(c in args.profile for c in '\n\r\0'):
        parser.error('Invalid Keychain profile name')
    settings = json.loads((ROOT / 'config/distribution.json').read_text())
    spec = importlib.util.spec_from_file_location('apple_signing_identity', ROOT / 'scripts/apple-signing-identity.py')
    identity = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(identity)
    try:
        # Fail before any build/upload if the correct Developer ID private key is absent.
        signer = identity.select(settings['team_id'], args.identity or os.environ.get('MSTEAMS_SIGNING_IDENTITY'))
        environment = dict(os.environ, MSTEAMS_SIGNING_IDENTITY=signer, APPLE_SIGNING_IDENTITY=signer)
        # Avoid Tauri's separate env-based automatic notarization; use only the
        # explicit Keychain profile below, with no passwords/API keys in argv.
        for key in ['APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID', 'APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH']:
            environment.pop(key, None)
        subprocess.run([str(ROOT / 'scripts/build-configurator.sh'), '--release'], env=environment, check=True)
        base = ROOT / 'src-tauri/target/release/bundle/macos'
        runtime, configurator = base / 'Microsoft Teams.app', base / 'Teams Configurator.app'
        verify_bundle(runtime, settings['runtime_identifier'], settings['team_id'])
        verify_bundle(configurator / 'Contents/Resources/RuntimeTemplate.app', settings['runtime_identifier'], settings['team_id'])
        verify_bundle(configurator, settings['configurator_identifier'], settings['team_id'])
        work = pathlib.Path(tempfile.mkdtemp(prefix='teams-notarization-'))
        os.chmod(work, 0o700)
        for app in [runtime, configurator]:
            notarize(app, args.profile, work)
            verify_bundle(app, settings['runtime_identifier'] if app == runtime else settings['configurator_identifier'], settings['team_id'])
            output = base.parent / 'distribution'
            output.mkdir(exist_ok=True)
            archive = output / (app.stem + '.zip')
            if archive.exists():
                archive.unlink()
            subprocess.run(['/usr/bin/ditto', '-c', '-k', '--keepParent', str(app), str(archive)], check=True)
            print('Distribution archive:', archive)
        print('Notarization receipts:', work)
        return 0
    except (ValueError, json.JSONDecodeError) as error:
        print(error, file=sys.stderr)
    except subprocess.SubprocessError:
        print('Apple signing/notarization command failed; credentials were not printed. Check the Keychain profile and submission status.', file=sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main())
