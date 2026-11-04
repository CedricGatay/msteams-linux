# Teams shell and configurator for macOS

An unofficial [Tauri v2](https://tauri.app) shell for the Microsoft Teams web
client, with native notifications, media permissions, tray integration and
optional Keychain-backed session restoration.

**The configurator is the only macOS release download. It creates independent,
fully bundled Teams `.app` applications from JSON. End users do not need Rust,
Node, Python or this checkout. No standalone Teams app is distributed.**

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

Use `./scripts/build-configurator.sh --release` for a release build. The runtime
is only a build intermediate: it is copied into the configurator as
`Contents/Resources/RuntimeTemplate.app`, then the standalone intermediate is
removed. Generated Teams apps are created exclusively through the configurator.

For the signed/notarized release, follow [the distribution workflow](docs/apple-distribution.md).
Its managed output directory contains only `Teams Configurator.zip`, `SHA256SUMS`
and `release.json`; the manifest records the source commit and architecture.

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
| `user_agent` | `"auto"` (default): Safari/WebKit identification on macOS, desktop Chrome on other platforms. An explicit string overrides it. |
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
Quit a generated app before updating it. To receive runtime features such as
zoom or Dock badges, regenerate/update it using the newer configurator template;
changing only its JSON does not update the executable. Use a new app for a different account
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
Per-app preferences (in the macOS **Help** menu) override configuration defaults. **No authentication
cookies, accounts, browser profiles or Keychain entries are exported.**

The same runtime can enter the local configurator with `--configure`. The
packaged configurator uses a role marker, so no command-line option is needed.

## Signing and launching

Bundles use the `org.9h41` namespace: `org.9h41.teams` for the runtime,
`org.9h41.teams.configurator` for the configurator, and isolated generated IDs
under `org.9h41.teams.generated`. Renaming creates a new macOS profile/permission
namespace; old authentication is not silently copied.

Local bundles are ad-hoc signed with matching `CFBundleIdentifier` and sealed
`Info.plist`/resources. Camera/audio-input entitlements preserve media access under
hardened runtime. Public distribution still requires the appropriate Developer ID
signing and notarization; local ad-hoc signing is not notarization.

For Developer ID signing and Apple notarization with the 9h41 account, see
[the team-checked distribution workflow](docs/apple-distribution.md). It requires
an installed 9h41 Developer ID Application certificate/private key and a
notarytool Keychain profile. Development certificates and other teams' identities
are deliberately rejected. Personalized configurator outputs remain locally
ad-hoc signed; no developer credentials ship with them.

Launch the configurator and generated apps through Finder or `open`, not their
raw executables. Quit a generated app via **Quit / ⌘Q** before updating it. The
build refuses to overwrite an active build-output bundle; it never force-quits
apps or copies authentication data into the template.

## Desktop features

- External links open in the default browser; configured trusted sign-in hosts
  stay in the shared WebView profile.
- Closing the main window hides it; the Dock icon or **Help → Show** restores it.
  ⌘Q, the application-menu Quit action, or **Help → Quit** exits cleanly.
- **View → Reload Teams (⌘R)** refreshes the Teams webview without restarting the
  process or clearing cookies/profile data. A blank document returns to the
  configured Teams start page; an active Keychain restoration is not interrupted.
  As with browser reload, unsaved page state may be reset.
- On macOS, the former tray commands now live in **Help** (window visibility,
  media permissions, notification tests, restoration preferences and diagnostics).
  The tray icon only restores the window when clicked. Other platforms retain
  their tray context menu.
- **View → Show Tray Icon** toggles the optional menu-bar status icon. This is
  remembered per app in `tray-visible.json` (default: visible). Help commands and
  Dock reopening continue to work when the icon is hidden; the Dock unread badge
  is independent of tray visibility.
- File drops in the main Teams window use WebKit's HTML5 `File`/`DataTransfer`
  handling instead of Tauri's native drop interception. Drop onto Teams' chat
  composer/drop zone; Teams controls upload, confirmation and tenant restrictions.
  The wrapper does not upload files itself or grant webpage filesystem access.
  Existing generated apps need a runtime update from a newer configurator;
  changing the JSON alone cannot change this native behavior.
- **View → Zoom In / Zoom Out / Actual Size** controls the Teams page. Shortcuts:
  ⌘+ (or ⌘=), ⌘− and ⌘0. Zoom uses 10-point steps from 50% to 200%, is saved
  per app in `zoom.json`, and is reapplied on Teams navigation. The configurator
  and sign-in pages stay at actual size. No arbitrary zoom value is accepted by
  page IPC.
- The unread count is synchronized between the menu-bar tray badge and the Dock
  app icon, including `99+` overflow and clearing at zero. Each generated app
  updates only its own Dock tile.
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
  shutdown and no recording. **Help → Enable microphone and camera** retries.
  OS privacy decisions still apply; screen capture is never requested on startup.

If a notification is accepted but no banner appears, check macOS Notifications,
alert style and Focus. API rejection from the native test shows a domain/code
error dialog without logging message contents. Teams' own settings preview uses
an in-app toast command, and browser delivery can be suppressed while focused:
see [source investigation and adapter details](docs/teams-notification-integration.md).

### Session restoration

Enable **Restore session on restart (Keychain)** or set its configuration default.
On clean Help/application-menu/⌘Q exit, only allowed Secure session cookies are
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
worker URLs or storage values. Diagnostic actions are in **Help** on macOS.
The trace-only **Send web-bridge test notification**
exercises page IPC, unlike the native-only test. `MSTEAMS_NATIVE_FOREGROUND=1`
enables the adapter for one run; `MSTEAMS_TEST_NOTIFICATION=1` sends a synthetic
native test on launch. Normal launcher runs clear temporary diagnostic flags.
Never direct private logs into the checkout.

### macOS browser identification and profile photos

The macOS runtime uses WebKit, not Chromium. With `user_agent: "auto"` it identifies
as Safari, using the installed Safari version (17.1 fallback if system metadata
is unavailable). This fixed a live case where Chrome identification produced
only default avatars and photo requests returned 401, while Safari identification
loaded the photos in the **same profile**, without weakening cookie/privacy
settings. The exact upstream cookie/feature-detection mechanism is not proven.
Custom user-agent strings are preserved; an older JSON explicitly advertising
Chrome must remove that override or use `"auto"` with the updated runtime. Browser
identification does not add engine features or bypass account policy.

**Save avatar diagnostics (metadata only)** takes a manual, read-only snapshot of
image/Fluent-avatar loaded, failed and pending counts, failed avatar URL-scheme
buckets, and photo-request HTTP status buckets exposed by resource timing. Missing
or cross-origin-hidden status is **unknown**, never treated as proof of a network
failure. Worker-only requests or requests outside the bounded timing buffer may
not appear. Only counts/booleans are written to `avatar-diagnostics.json` in that
app's Application Support directory (owner-only permissions). No URLs, names,
headers, cookies, tokens, pixels or image bodies are saved. It does not retry
requests, change image loading or alter authentication. This is diagnostic
instrumentation, not an avatar compatibility fix. The probe also reports the
presence of the photo provider's base URL and its cookie-validity **boolean**;
it never records the URL or cookie itself. React/provider discovery is bounded,
uses only named photo-state keys, and does not invoke arbitrary getters or alter
provider state.

For native startup regressions on macOS, run:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --release \
  --features tauri/custom-protocol --example startup-smoke
```

This hidden, incognito smoke test verifies that a synthetic Objective-C exception
is caught in **release mode**, that native Help/View controls are constructed, and
that an embedded data document actually loads and reloads.
It does not access a user profile or make network requests. macOS releases must
use `panic = "unwind"`; `panic = "abort"` disables the Objective-C exception
catching that WebKit URL-scheme cancellation relies on. The restoration bootstrap
also requires Tauri's `webview-data-url` feature, enabled in `Cargo.toml`.

Only the compiled, read-only diagnostic runs. Experimental local-script loading
and one-off authenticated photo probes have been removed.

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
The sanitized history is published to [the personal fork](https://github.com/CedricGatay/msteams-linux).
Avoid merging an unsanitized old branch back into it. Private configuration,
authentication data and migration backups remain outside the repository.
