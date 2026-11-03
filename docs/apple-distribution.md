# 9h41 macOS distribution

## Ownership and identities

- Developer team: `P222GS5YTA` (9h41).
- Runtime bundle: `org.9h41.teams`.
- Configurator bundle: `org.9h41.teams.configurator`.
- New personalized apps: `org.9h41.teams.generated.app<random UUID>`.

Public ownership metadata lives in `config/distribution.json`. Certificates,
private keys, Apple IDs, app-specific passwords and App Store Connect API keys
do **not** belong in this repository or in an exported app.

Changing a bundle ID creates a separate app-data, WebKit and Keychain namespace.
Existing authentication is not copied automatically between identities. A local
migration can copy only non-secret connection settings/preference booleans into
the new private Application Support folder while leaving the old profile intact;
expect fresh OS permissions and one normal sign-in for the renamed app.

## Required credentials

Notarized distribution outside the Mac App Store requires a valid **Developer ID
Application** certificate **and its private key** for the team above. An **Apple
Development** or **Apple Distribution** certificate is not an interchangeable
substitute. Do not select an unrelated team's Developer ID just because it is
installed.

Create/import the Developer ID Application certificate through the authorized
9h41 developer account using Apple's developer tools/portal and Keychain Access.
Keep any private-key backup outside the checkout. Verify availability:

```bash
security find-identity -v -p codesigning
python3 scripts/apple-signing-identity.py
```

The selector prints only the matching certificate fingerprint. It checks valid
identity/private-key availability, certificate type and Team ID, including the
public certificate OU. If several matching identities exist, provide the desired
fingerprint using `--identity` or `MSTEAMS_SIGNING_IDENTITY`.

Store notarization authentication in Keychain, not in shell scripts or config:

```bash
xcrun notarytool store-credentials "9h41-notary" \
  --apple-id "YOUR_APPLE_ID" --team-id P222GS5YTA
```

Leave `--password` absent so Apple's tool uses its secure interactive prompt.
Alternatively provision an appropriate App Store Connect API-key profile through
Apple's tooling, keeping the key outside the repo. Never paste secrets into chat.
The profile name is a reference, not a credential value.

## Build and notarize

```bash
python3 scripts/notarize-macos.py --profile "9h41-notary"
```

The workflow:

1. Fails before build/upload if the correct Developer ID identity is missing.
2. Builds release runtime and configurator with the same selected identity.
   The embedded runtime template is signed before the outer configurator.
3. Verifies all three bundle signatures, expected bundle IDs, expected team,
   Developer ID authority, hardened runtime and secure timestamps.
4. Zips the two public bundles using `ditto`, submits each through `notarytool`
   using only the Keychain profile, and requires `Accepted` status.
5. Staples and validates each ticket, checks Gatekeeper assessment and rechecks
   signing identity after stapling.
6. Writes final stapled distribution ZIPs under
   `src-tauri/target/release/bundle/distribution/`.

Submission receipts/logs are kept in a private temporary directory outside the
checkout. Rejected or incomplete notarization is a failure, never reported as
successful. The tool does not disable Gatekeeper, change Keychain trust, or make
an unsupported claim that local ad-hoc builds are notarized.

For a Developer-ID-signed build without submission:

```bash
MSTEAMS_SIGNING_IDENTITY="CERTIFICATE_SHA1_FINGERPRINT" \
  ./scripts/build-configurator.sh --release
```

Without a selected identity, the ordinary developer build still uses local
ad-hoc signing. Such a build is **not** a notarized distribution artifact.

## Personalized outputs

The configurator must work on end-user machines without access to the developer
account. It therefore continues to ad-hoc sign generated/personalized apps.
Modifying identifiers/resources changes the signed product; the template's
Developer ID signature/notarization does **not** transfer to a personalized app.
Distributing those apps requires a separate authorized signing/notarization
process. Never embed a Developer ID private key or notary credentials to avoid
that requirement.

## Checks

```bash
python3 -m unittest discover -s scripts -p 'test_*.py' -v
```

These policy tests are offline and inspect synthetic public metadata only. They
reject wrong-team, development, ad-hoc, non-hardened and unstamped signatures.
Live Apple acceptance still requires the installed credentials and an actual
notarization receipt; policy/unit tests alone do not prove notarization.
