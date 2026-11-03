# Microsoft Teams — Tauri desktop wrapper

A tiny native shell around the [Microsoft Teams](https://teams.microsoft.com) web
client, built with [Tauri v2](https://tauri.app). It gives you a real desktop
window, a proper app icon, persistent sign-in, in-app authentication popups and
hands external links off to your browser — without bundling the official
(electron-based) Teams client.

> Unofficial project. Not affiliated with, endorsed by or supported by Microsoft.

## Features

- **Native window** that loads `https://teams.cloud.microsoft/v2/` directly,
  avoiding the legacy `teams.microsoft.com` entry-point redirect.
- **External links** (`http`/`https` to non-Microsoft hosts) open in your default
  browser instead of taking over the app.
- **In-app auth popups** — OAuth windows opened by Teams (`window.open`,
  `target="_blank"` to `login.microsoftonline.com`, …) are opened as separate
  app windows so the sign-in flow completes inside the wrapper. Organization
  SSO at `https://login.example.com`, `https://identity.example.com`, and the session
  control proxy `https://tenant-proxy.example.net` also stays
  in-app. Only those exact HTTPS hosts are allowed, not other `example.com` hosts
  or other tenants under `access.mcas.ms`. Session-control policies still apply.
- **Media & notifications** — camera, microphone, screen-share, media keys and
  notification permissions are granted automatically so notifications work and
  the call UI can request devices.

  > Calls/messages only actually connect on Linux where the system WebKitGTK was
  > built with WebRTC. See [Calls / WebRTC on Linux](#calls--webrtc-on-linux).
- **Native notifications** — Teams' web notifications are bridged to desktop
  notifications through `tauri-plugin-notification`.
- **Tray icon with unread badge** — closing the window hides the app to the
  system tray, whose icon shows a red badge with the unread message count
  (driven by Teams' web notifications and the unread counter it puts in
  `document.title`). Left-clicking the tray icon restores the window,
  right-clicking opens the menu.
- **Persistent profile** — on Linux/Windows the WebView data directory lives
  under the app data directory. On macOS 14+ all windows use one fixed WebKit
  data-store identifier (older macOS uses WebKit's default persistent store).
  Cookie/token expiry and organization-enforced sign-in frequency still apply.
- **Third-party cookies on Linux** — WebKitGTK's default cookie accept policy is
  *exclusively from the main document's domain*, which drops the cookies Teams
  uses during sign-in. The wrapper switches the policy to *always* (and turns
  Intelligent Tracking Prevention off) before the first request leaves, so
  Teams' third-party-cookie probe succeeds.
- **Browser identification** — main and sign-in windows use a desktop Chrome
  user agent to avoid Teams rejecting the default embedded-WebView identity.
  This does not turn WebKit into Chromium, add missing browser features, or
  bypass organization authentication / device-compliance requirements.
- **Cross-platform** — Linux, macOS and Windows (Tauri v2 runtime).

## Why the "third-party cookies" error page used to appear

Teams checks third-party cookie support at start-up by loading
`https://teams.public.onecdn.static.microsoft/…/report-third-party-cookies.html`
in a **hidden iframe** and asking that page to set a `SameSite=None` cookie. If
the probe fails, Teams immediately navigates to the very same URL — which is the
error dialog (and the MS link opened in your browser) you saw.

WebKitGTK emits its `decide-policy` signal for **every frame**, and Tauri's
`on_navigation` is a thin wrapper around it. The wrapper used to treat the probe
iframe exactly like a user-clicked link: because the OneCDN host was not in
`TRUSTED_HOSTS` the navigation was rejected *and* handed to the system browser, so
the probe never ran and Teams concluded the browser was broken.

The fix has three parts:

1. on Linux the navigation policy is installed directly on the WebKitGTK web
   view (`install_navigation_policy`) instead of going through `on_navigation`,
   and it only applies to **user-initiated** navigations (link clicks and form
   submissions). Sub-frames, redirects and script-driven loads — which
   WebKitGTK does not let us tell apart — are always left alone;
2. `static.microsoft` (the OneCDN bucket Teams serves itself from) is part of
   `TRUSTED_HOSTS`, so Teams' own report pages stay inside the app;
3. the third-party cookie settings are applied *before* the window navigates to
   Teams. Navigation waits until WebKitGTK's network process confirms the new
   cookie policy, so the first cookie probe cannot race the asynchronous update.

## Layout

```
.
├── .github/workflows/release.yml  # CI: checks + AppImage -> GitHub release
├── dist/                     # minimal placeholder page (bundled, unused at runtime)
├── package.json              # Tauri CLI scripts
├── package-lock.json         # pinned npm deps (required by CI's `npm ci`)
├── scripts/
│   ├── setup-sysroot.sh      # root-less build deps (fallback, see below)
│   ├── build.sh              # build/run helper that wires up the sysroot
│   ├── build-webrtc-webkit.sh # local WebKitGTK with WebRTC and media support
│   └── build-appimage.sh     # Linux AppImage packaging for the sysroot setup
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs              # declares the app commands for the Tauri ACL
    ├── tauri.conf.json       # app metadata, bundle + window config
    ├── capabilities/default.json
    ├── icons/                 # generated app icons
    └── src/
        ├── main.rs
        └── lib.rs            # all the wrapper logic
```

## Prerequisites

- [Rust](https://rustup.rs) (stable) and Cargo.
- [Node.js](https://nodejs.org) 18+ (only for the Tauri CLI).
- The Tauri **system dependencies** for your platform, most importantly
  `webkit2gtk-4.1` and `gtk3` on Linux. On Debian/Ubuntu/Mint:

  ```bash
  sudo apt update
  sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
      libjavascriptcoregtk-4.1-dev build-essential curl wget file libssl-dev \
      librsvg2-dev
  ```

See the [Tauri prerequisites guide](https://tauri.app/start/prerequisites/) for
macOS and Windows.

### No root / locked-down machine?

If you cannot install the `-dev` packages above, this repo can build a local
sysroot without root using `apt-get download`:

```bash
./scripts/setup-sysroot.sh   # once, downloads/extracts the -dev packages locally
./scripts/build.sh           # build (wires up the sysroot automatically)
```

`build.sh` falls back to system packages automatically when the local sysroot is
absent, so it is safe to use everywhere.

## Calls / media access on macOS

WebKit requires camera and microphone usage descriptions in the app's
`Info.plist`. Without them the WebView can expose `RTCPeerConnection` while
`navigator.mediaDevices.getUserMedia` is unavailable, causing Teams to report
that calls cannot be made or received. `src-tauri/Info.plist` declares both uses.

The main Teams page requests microphone and camera access once per document
load, including after returning from sign-in. It requests each separately and
immediately stops the test stream; no audio/video is recorded or sent. SSO pages
and auth popups do not request capture access. Screen capture is never requested
automatically. Use the tray menu's **Enable microphone and camera** to retry
without signing in again. If macOS has already denied camera access, allow it
under **System Settings → Privacy & Security → Camera**; a WebView request cannot
force macOS to show the system prompt again.

The fixed macOS 14+ WebKit profile replaces the previous default profile, so the
first launch after this change requires signing in once. Subsequent launches
reuse that profile. This does not override session expiry or organization
policies. Quit via the tray menu before testing persistence across relaunches.

Build and launch the app bundle, and allow macOS camera/microphone prompts:

```bash
npm run tauri -- build --debug --bundles app
open "src-tauri/target/debug/bundle/macos/Microsoft Teams.app"
```

With `MSTEAMS_DEBUG=1`, the app logs a read-only media capability probe when
Teams loads. It does not activate the microphone or camera. API availability
alone does not verify end-to-end calling or organization policy compatibility.

## Optional session restoration on macOS

WebKit drops session-only cookies when the app exits. To retain those across a
clean restart, right-click the tray icon and enable **Restore session on restart
(Keychain)**. This is **off by default** for new installations. Enable it before
quitting a signed-in session; the first successful capture requires signing in
normally. No browser cookies or accounts are imported.

- On a clean Quit (tray or application-menu/⌘Q), Secure session cookies from the exact
  Teams and organization SSO hosts are saved in macOS Keychain. No cookie/token
  values are written to files, diagnostic output, or command-line arguments.
- The snapshot is restored before the main Teams navigation. Existing cookies
  are not overwritten. Domain, path, Secure, HttpOnly and SameSite attributes
  are round-tripped through the native cookie API; no expiry is added or changed.
- Snapshots are scoped to this app's fixed WebKit profile, limited to 24 hours,
  and consumed on use. A fresh snapshot is captured on the next clean Quit.
  This is session restoration, not permanent token storage; server expiry,
  sign-out and revocation still apply. It does not restore tab `sessionStorage`.
- Startup falls back to normal sign-in after at most four seconds of restoration
  work; Quit waits at most five seconds for capture. Locked/denied Keychain access
  or a forced process termination may mean signing in again. The macOS application
  menu's custom ⌘Q action is routed through capture; native Dock Quit and system
  termination can bypass it, so use the tray or application menu for restoration.
- **Disable session restore and forget saved session** stops restoration and
  requests deletion of the Keychain snapshot, without signing out the currently
  running app. Keychain access may prompt after a rebuild or if locked. If
  deletion is denied, the entry can be removed in Keychain Access under
  `com.github.msteams.desktop.session-restore`; disabled restoration will not use it.

Only the opt-in boolean is saved outside Keychain, in
`session-restore-enabled.json` under the app data directory. Linux and Windows
retain their existing WebView behavior; this feature is macOS-only.

## Diagnosing repeated sign-in

A persistent profile does not necessarily restore session-only cookies or tab
`sessionStorage` like a browser's session-restore feature does. The tray's
**Save session diagnostics (metadata only)** action saves a best-effort
`session-diagnostics-manual.json` snapshot in the app data directory
(`~/Library/Application Support/com.github.msteams.desktop` on macOS).
Diagnostics are manual-only: neither startup nor Quit waits for a diagnostic
JavaScript callback. Save a copy before requesting another snapshot if comparing
across restarts.

Snapshots contain session/persistent/expired cookie counts for fixed sign-in
hosts and local/session storage key counts (including MSAL-related key counts).
They never contain cookie values, cookie names, storage values, storage keys,
or token exports. They do not change cookie expiry or restore authentication
state. Compare snapshots from the same build before drawing conclusions about
WebKit versus server-enforced authentication.

## Calls / WebRTC on Linux

Teams calls and meetings need the browser `RTCPeerConnection` API. The wrapper
asks WebKitGTK for it (`enable-webrtc` in `configure_webkit`), but that setting
is a **no-op unless the installed WebKitGTK was compiled with `ENABLE_WEB_RTC`**.
Ubuntu/Debian ship `libwebkit2gtk-4.1-0` with that option off, so
`RTCPeerConnection` is `undefined` and Teams shows:

> Something went wrong — You can't receive or make calls right now. Please try
> again by restarting the app.

Two independent reasons make this a system-level problem rather than an app bug:

- the GTK port defaults `ENABLE_WEB_RTC` to `ENABLE_EXPERIMENTAL_FEATURES`,
  which is off in the distro build (EME and WebGPU are off for the same reason);
- the `enable-webrtc` setting only flips a per-page WebCore setting, so it can
  never add a feature that was not compiled in.

Verify it directly:

```bash
# in the app console (MSTEAMS_DEBUG=1) or a WebKitGTK test page
typeof RTCPeerConnection   // "undefined" on stock Ubuntu WebKitGTK
```

The wrapper cannot add WebRTC to an already installed WebKitGTK, so the project
builds a compatible local copy instead of hiding the calling error:

```bash
./scripts/build-webrtc-webkit.sh  # once; a large local build, no root required
./scripts/build.sh --run
```

The build enables `ENABLE_MEDIA_STREAM`, `ENABLE_WEB_RTC` and the GStreamer
WebRTC backend. `build.sh` automatically points both the app and WebKit's helper
processes at this local copy. The AppImage build packages the same copy so calls
also work without replacing the system's WebKitGTK. Camera and microphone access
must be allowed by the desktop environment. For running from source on
Debian/Ubuntu/Mint, install the media runtime plugins:

```bash
sudo apt install gstreamer1.0-plugins-bad gstreamer1.0-plugins-good \
    gstreamer1.0-alsa gstreamer1.0-gl gstreamer1.0-pipewire \
    gstreamer1.0-pulseaudio
```

To verify the runtime without calling a Teams contact, run the WebRTC smoke
example. It opens the camera and microphone, negotiates two local peer
connections, and requires ICE, inbound audio/video RTP and successful playback
before passing. No recording is saved and no media leaves the machine.

```bash
./scripts/build.sh --run --example webrtc-smoke
```

## AppImage

```bash
./scripts/build-appimage.sh   # produces Microsoft_Teams-x86_64.AppImage
./Microsoft_Teams-x86_64.AppImage --webrtc-smoke  # synthetic-media runtime check
```

The packaged smoke check uses generated audio/video instead of hardware devices,
but exercises the same real WebRTC, ICE, codecs and decoders. The release
workflow runs it in a virtual display before publishing an AppImage.

This is a hand-driven packaging run because the upstream `tauri build
--bundles appimage` conflicts with the local sysroot (its gtk plugin asks
pkg-config, which returns sysroot-prefixed paths, for the runtime data
directories). The script instead calls linuxdeploy with a clean environment and
ships the pieces the webkit gtk plugin does not handle itself:

- `~/.cache/tauri/AppRun-x86_64` — the Tauri AppRun, wrapped by a small launcher
  that points WebKit at paths relative to the mounted AppImage,
- the local WebKitGTK/JavaScriptCore libraries, helper processes and
  injected-bundle, with no dependency on the distro WebKit build,
- the GStreamer runtime plugins, including WebRTC and audio/video support,
- `libayatana-appindicator3` (loaded by `dlopen`, invisible to linuxdeploy's
  ldd scan) with its full dependency closure.

Run `npm run tauri build -- --bundles appimage` once first if the linuxdeploy /
AppRun cache is missing (it may fail at the gtk step; that populates the cache).

### GitHub Actions

`.github/workflows/release.yml` builds and publishes the AppImage:

| Trigger | What happens |
| --- | --- |
| push tag `v*` | `fmt`/`clippy`/`tests` → build AppImage → publish a GitHub release for that tag |
| **Actions → Run workflow** | same, published as a prerelease; fill the optional `tag` input to build *and* check out that tag |
| pull request touching `src-tauri/**`, `dist/**`, `scripts/**`, `package*.json` | checks only, nothing is published |

The release job runs the same `build-webrtc-webkit.sh` and `build-appimage.sh`
scripts as a local build. The resulting file is renamed to
`Microsoft_Teams-<version>-x86_64.AppImage` and uploaded as a release asset.

Rust, npm, WebKitGTK and the `~/.cache/tauri` AppImage tooling are cached between
runs.
`contents: write` is granted to the `publish` job only.

Publishing needs no extra setup beyond the default `GITHUB_TOKEN`, since the
workflow pushes to the same repository it is triggered from.

## Build & run

Using the Tauri CLI (the normal workflow):

```bash
npm install
npm run dev       # dev build + run
npm run build     # production bundles
```

Or with the helper script:

```bash
./scripts/build.sh            # cargo build (debug)
./scripts/build.sh --release  # cargo build --release
./scripts/build.sh --run      # build + launch
```

## Configuration

Almost everything lives in `src-tauri/src/lib.rs`:

- `TEAMS_URL` — the entry point loaded at startup.
- `TRUSTED_HOSTS` — the **allow-list** of suffixes that may load inside the app.
  Links to anything else are opened externally. Tighten/extend this if your tenant
  uses custom domains.
- `create_window` — window size, title and the navigation / popup / permission
  handlers shared by the main window and popups. The window is built on
  `about:blank` and navigated to Teams afterwards, so the WebKitGTK settings are
  in place before the first request.
- `configure_webkit` (Linux only) — reaches into the WebKitGTK webview via
  `with_webview`, turns Intelligent Tracking Prevention off and sets the cookie
  accept policy to *always* (without them Teams shows its third-party-cookie
  error page), enables `MediaCapabilities` and asks for WebRTC support (see
  [Calls / WebRTC on Linux](#calls--webrtc-on-linux)). With `MSTEAMS_DEBUG=1`
  it also prints the resulting ITP and cookie-policy state.
- `install_navigation_policy` (Linux only) — the "open external links in the
  browser" rule, implemented on the WebKitGTK webview and restricted to
  user-initiated navigations (see
  [Why the "third-party cookies" error page used to appear](#why-the-third-party-cookies-error-page-used-to-appear)).
- `close_popups` — drops the leftover `auth-*` windows after sign-in.
- `permission_response` — which permissions are auto-granted.
- `INJECT_JS` — script injected into the Teams pages: it hooks the web
  `Notification` API, watches `document.title` for the `(N)` unread counter and
  forwards both to the `set_unread_count` / `show_notification` commands.
- `render_badge` — composites the red unread badge onto the tray icon.
- The tray, its menu and the hide-to-tray close behaviour are set up in `run()`.

The `set_unread_count` / `show_notification` commands are app commands: they are
declared in `src-tauri/build.rs` (which generates their `allow-*` permissions)
and granted by `src-tauri/capabilities/default.json`, together with the
Microsoft origins the app is allowed to talk to.

App metadata (name, identifier, icons, bundle targets) is in
`src-tauri/tauri.conf.json`.
