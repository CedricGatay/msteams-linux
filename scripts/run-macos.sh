#!/usr/bin/env bash
# Launch the signed bundle through Launch Services, never its raw executable.
# Quit via Cmd-Q first when switching builds; this script never kills or rebuilds.
# Optional: MSTEAMS_DEBUG=1 MSTEAMS_TEST_NOTIFICATION=1 MSTEAMS_LOG_FILE=/tmp/teams.log
# Metadata-only browser-to-native tracing: MSTEAMS_NOTIFICATION_TRACE=1
# Opt in to the experimental foreground adapter for this run: MSTEAMS_NATIVE_FOREGROUND=1
# Use MSTEAMS_BUILD_PROFILE=release for a release bundle (default: debug).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${MSTEAMS_BUILD_PROFILE:-debug}"
case "$PROFILE" in
  debug|release) ;;
  *) echo "MSTEAMS_BUILD_PROFILE must be debug or release" >&2; exit 2 ;;
esac
APP="$ROOT/src-tauri/target/$PROFILE/bundle/macos/Microsoft Teams.app"
if [[ "$(uname -s)" != Darwin || ! -d "$APP" ]]; then
  echo "Requires macOS and a built app bundle. Build: npm run tauri -- build --debug --bundles app" >&2
  exit 1
fi

/usr/bin/codesign --verify --deep --strict "$APP"
IDENTIFIER="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist")"
SIGNATURE="$(/usr/bin/codesign -dv --verbose=2 "$APP" 2>&1)"
if ! grep -Fxq "Identifier=$IDENTIFIER" <<<"$SIGNATURE" || ! grep -q '^Info.plist entries=' <<<"$SIGNATURE"; then
  echo "Bundle signature must bind Info.plist and match CFBundleIdentifier; rebuild the signed bundle." >&2
  exit 1
fi

# Explicit defaults avoid carrying a previous diagnostic launch into a new run.
OPEN_ARGS=(-a "$APP" --env "MSTEAMS_TEST_NOTIFICATION=${MSTEAMS_TEST_NOTIFICATION:-0}" --env "MSTEAMS_NOTIFICATION_TRACE=${MSTEAMS_NOTIFICATION_TRACE:-0}" --env "MSTEAMS_NATIVE_FOREGROUND=${MSTEAMS_NATIVE_FOREGROUND:-0}")
if [[ -n "${MSTEAMS_DEBUG:-}" ]]; then
  OPEN_ARGS+=(--env "MSTEAMS_DEBUG=$MSTEAMS_DEBUG")
fi
if [[ -n "${MSTEAMS_LOG_FILE:-}" ]]; then
  OPEN_ARGS+=(--stderr "$MSTEAMS_LOG_FILE")
fi
exec /usr/bin/open "${OPEN_ARGS[@]}"
