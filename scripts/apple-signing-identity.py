#!/usr/bin/env python3
"""Select a valid Developer ID Application identity for an explicitly chosen team.
Reads public certificate metadata only; never exports or prints a private key.
"""
import argparse
import pathlib
import re
import subprocess
import sys


def candidates(text, team):
    found = []
    for fingerprint, label in re.findall(r'\b([A-Fa-f0-9]{40})\s+"([^"\n]+)"', text):
        if label.startswith('Developer ID Application: ') and label.endswith(f'({team})'):
            found.append(fingerprint.upper())
    return found


def select(team, preferred=None):
    result = subprocess.run(['security', 'find-identity', '-v', '-p', 'codesigning'], capture_output=True, text=True, check=True)
    choices = candidates(result.stdout, team)
    if preferred:
        preferred = preferred.upper()
        if preferred not in choices:
            raise ValueError('Requested identity is not a valid Developer ID Application identity for the configured team')
        choices = [preferred]
    if not choices:
        raise ValueError('No valid Developer ID Application certificate/private key is available for the configured team')
    if len(choices) > 1:
        raise ValueError('Multiple matching identities; select a fingerprint using MSTEAMS_SIGNING_IDENTITY')
    # Confirm the certificate OU, rather than relying solely on its display label.
    certificates = subprocess.run(['security', 'find-certificate', '-a', '-c', 'Developer ID Application', '-p'],
                                  capture_output=True, text=True, check=True).stdout
    for pem in re.findall(r'-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----', certificates, re.S):
        fingerprint = subprocess.run(['openssl', 'x509', '-noout', '-fingerprint', '-sha1'], input=pem,
                                     capture_output=True, text=True, check=True).stdout
        fingerprint = fingerprint.split('=', 1)[-1].replace(':', '').strip().upper()
        if fingerprint != choices[0]:
            continue
        subject = subprocess.run(['openssl', 'x509', '-noout', '-subject', '-nameopt', 'RFC2253'], input=pem,
                                 capture_output=True, text=True, check=True).stdout.strip()
        if re.search(r'(?:^|,)OU=' + re.escape(team) + r'(?:,|$)', subject.removeprefix('subject=').strip()):
            return choices[0]
    raise ValueError('Signing certificate team could not be verified')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--team-id')
    parser.add_argument('--identity')
    args = parser.parse_args()
    team = args.team_id
    if not team:
        import json
        team = json.loads((pathlib.Path(__file__).resolve().parents[1] / 'config/distribution.json').read_text())['team_id']
    if not re.fullmatch(r'[A-Z0-9]{10}', team):
        parser.error('Invalid Apple Team ID')
    try:
        print(select(team, args.identity))
    except (ValueError, subprocess.SubprocessError) as error:
        print(str(error) if isinstance(error, ValueError) else 'Cannot inspect local signing identities', file=sys.stderr)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main())
