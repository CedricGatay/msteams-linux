# Teams shell and configurator for macOS

An unofficial [Tauri v2](https://tauri.app) shell for the Microsoft Teams web
client, with native notifications, media permissions, tray integration and
optional Keychain-backed session restoration.

**The configurator creates independent, fully bundled macOS `.app` applications
from a JSON file. End users do not need Rust, Node, Python or this checkout.**

Not affiliated with or supported by Microsoft. Configuration files choose which
websites receive native integrations: use only configurations you trust.

## Build the configurator

Developer prerequisites: macOS 11+, Rust, Node/npm and Apple's command-line build
tools. The generated runtime targets the same architecture as the template.

```bash
npm ci
./scripts/build-configurator.sh
open "src-tauri/target/debug/bundle/macos/Teams Configurator.app"
```

Use `./scripts/build-configurator.sh --release` for release artifacts. The build
produces a generic runtime plus a configurator containing a copy of that runtime
as `Contents/Resources/RuntimeTemplate.app`.

The configurator opens the native **Open configuration file** panel immediately.
Cancelling leaves a usable screen with a retry button. Select a valid file, enter
an app display name, choose **Create app**, and select an output location such as
`~/Applications`. **Personal configurations and generated apps must be outside
this checkout.** The exporter rejects destinations inside it.

The resulting app contains its own executable, assets, metadata and embedded
settings. It runs without the configurator installed. **Open generated app**
launches the completed bundle through Launch Services.

## Configuration

`config/default.json` contains generic defaults; `config/example.json` contains
only RFC-reserved example domains. Real organization settings belong in private
app storage or a trusted external file, never in source, tests, docs or Git
history. Do not put credentials, tokens or private configuration in this repo,
even in ignored files.

A minimal external file can override only what is needed:

```json
{
  "version": 1,
  "name": "Teams Example",
  "teams_url": "https://teams.cloud.microsoft/v2/",
  "trusted_auth_hosts": ["login.example.com", "identity.example.net"],
  "session_cookie_hosts": [
    "login.microsoftonline.com", "teams.cloud.microsoft", "teams.microsoft.com",
    "login.example.com", "identity.example.net"
  ],
  "restore_session": true,
  "native_foreground": false
}
```

| Field | Meaning |
|---|---|
| `version` | Schema version, currently `1`. |
| `name` | Display name, without paths or `.app` suffix. Editable in the UI. |
| `teams_url` | Trusted HTTPS landing page, without credentials, query or fragment. |
| `user_agent` | Browser identification; does not add Chromium features to WebKit. |
| `trusted_host_suffixes` | Navigation suffix allowlist; generic Microsoft defaults apply when omitted. |
| `trusted_auth_hosts` | Additional **exact HTTPS hosts**, not wildcard parent domains. |
| `notification_origins` | Exact trusted HTTPS origins allowed to use native notification/media hooks. |
| `session_cookie_hosts` | Exact trusted hosts whose Secure session cookies may be restored. |
| `restore_session` | Default opt-in for Keychain session restoration; default `false`. |
| `native_foreground` | Default opt-in for the experimental Teams foreground adapter; default `false`. |

Unknown fields, oversized files, malformed hosts, wildcard host entries, URL
credentials and inconsistent identities are rejected. Lists override their
default list rather than being appended. Native notification origins receive
only the main-window notification/unread commands; navigation trust does not
automatically grant IPC access to authentication pages.

`identifier`, `profile_id` and `generated` are embedded runtime metadata. New
exports always assign fresh identities, regardless of identity values in an
input file. When updating a previously generated app, the output app's existing
bundle ID/profile UUID are retained. The exporter refuses unrelated apps,
running generated apps and symlinked templates/targets. It stages, signs and
verifies before replacement; a signing failure leaves the existing app intact.
Quit a generated app before updating it. Use a new app for a different account
or organization rather than sharing a profile.

### Runtime loading

Before Tauri derives its data paths, the runtime loads settings in this order:

1. An explicit `--config /outside/settings.json` (developer/testing mode).
2. `Contents/Resources/runtime-config.json` in a generated bundle.
3. A private `runtime-config.json` in the bundle ID's Application Support folder.
4. Generic built-in settings.

On macOS this folder is `~/Library/Application Support/<bundle-id>/`. An invalid
configuration opens the local configurator with an error instead of loading an
unintended remote page. Runtime identity must match the actual app bundle.
The original shell retains its legacy profile when using its private settings;
newly generated apps have independent bundle IDs, WebKit profiles, data directories
and Keychain service/account namespaces. Updates preserve those namespaces.
Per-app tray preferences override configuration defaults. **No authentication
cookies, accounts, browser profiles or Keychain entries are exported.**

The same runtime can enter the local configurator with `--configure`. The
packaged configurator uses a role marker, so no command-line option is needed.

## Signing and launching

Local bundles are ad-hoc signed with matching `CFBundleIdentifier` and sealed
`Info.plist`/resources. Camera/audio-input entitlements preserve media access under
hardened runtime. Public distribution still requires the appropriate Developer ID
signing and notarization; local ad-hoc signing is not notarization.

Launch bundled apps through Finder or `open`, not their raw executables. For the
original developer runtime:

```bash
npm run tauri -- build --debug --bundles app
./scripts/run-macos.sh
```

The launcher verifies the signature and uses Launch Services. Set
`MSTEAMS_BUILD_PROFILE=release` to use the release bundle. It never force-quits or
rebuilds an active app. Quit via the application's **Quit / ⌘Q** action before
switching builds.

## Desktop features

- External links open in the default browser; configured trusted sign-in hosts
  stay in the shared WebView profile.
- Closing the main window hides the app to the tray; ⌘Q or tray Quit exits it.
- Native notifications use `UNUserNotificationCenter` under the actual bundle's
  identity. The first Teams page asks for OS authorization when undecided.
  **Send test notification** tests the native backend; click brings Teams forward.
- The standard page bridge handles `Notification` and page-side
  `ServiceWorkerRegistration.showNotification`, tag replacement, silent messages,
  clicks and close. Worker-only globals/push after full Quit are not implemented.
- The experimental **Also show native notifications while focused** option
  adapts only Teams candidates returning `skippedAppInFocus`, after original
  permission/settings checks. It preserves preview redaction, activation and
  deduplication. Submitted foreground banners survive the in-app toast's early
  close; macOS owns dismissal, with bounded callback retention. Normal browser
  close/cancellation and error cleanup remain unchanged. Internal service lookup
  is guarded and falls back safely after incompatible Teams updates.
- Camera/microphone are requested separately on Teams load, with immediate stream
  shutdown and no recording. The tray **Enable microphone and camera** retries.
  OS privacy decisions still apply; screen capture is never requested on startup.

If a notification is accepted but no banner appears, check macOS Notifications,
alert style and Focus. API rejection from the native test shows a domain/code
error dialog without logging message contents. Teams' own settings preview uses
an in-app toast command, and browser delivery can be suppressed while focused:
see [source investigation and adapter details](docs/teams-notification-integration.md).

### Session restoration

Enable **Restore session on restart (Keychain)** or set its configuration default.
On clean tray/application-menu/⌘Q exit, only allowed Secure session cookies are
saved in macOS Keychain. Startup shows a local restoration screen and allows up
to two minutes for approval, with **Continue without restoring** available.
Late/cancelled reads do not consume an unused snapshot; quitting during approval
preserves it. No JavaScript callback gates restoration. Shutdown capture is
bounded to five seconds. Native Dock/system termination may bypass capture.

Snapshots are profile- and bundle-scoped, limited to 24 hours, consumed before
use, and never written in plaintext or given rewritten cookie expiries. Server
revocation/session rules still apply. **Disable session restore and forget saved
session** stops restoration and requests deletion without signing out the live
window. Updates may require Keychain approval because local signatures change.

## Diagnostics and tests

Temporary metadata tracing, after a clean Quit:

```bash
MSTEAMS_NOTIFICATION_TRACE=1 MSTEAMS_LOG_FILE=/tmp/teams-trace.log ./scripts/run-macos.sh
```

This records fixed stage names, permission values, focus/visibility, counters,
hook status and bounded event histories—not titles, bodies, DOM text, IDs,
worker URLs or storage values. The trace-only **Send web-bridge test notification**
exercises page IPC, unlike the native-only test. `MSTEAMS_NATIVE_FOREGROUND=1`
enables the adapter for one run; `MSTEAMS_TEST_NOTIFICATION=1` sends a synthetic
native test on launch. Normal launcher runs clear temporary diagnostic flags.
Never direct private logs into the checkout.

**Save session diagnostics** records allowed-host cookie lifetime counts and
storage-key counts in private app storage, never authentication values. Native
media diagnostics (`MSTEAMS_DEBUG=1`) report capabilities/device counts, not
names/identifiers or captured media.

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked --lib
node --test scripts/*.test.cjs
cargo clippy --manifest-path src-tauri/Cargo.toml --locked --lib --examples -- -D warnings
```

The optional native Keychain smoke is ignored by default and uses synthetic data.
`examples/export-app.rs` exercises packaging outside the checkout with a prebuilt
template; a bundled runtime's `--check-config` reports only role/identity/profile
metadata and exits before network, media or Keychain access.

## Repository privacy and history

Only generic configuration belongs here. User-specific settings live outside
this checkout and are not compiled into public templates. Existing local history
was sanitized to remove organization-specific references; old commit IDs changed.
No remote push/force-push was performed. Avoid merging an unsanitized old branch
back into this history. Private migration backups are outside the repository.
