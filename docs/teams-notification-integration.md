# Teams notification integration: source investigation

## Scope and evidence

The shell's native backend and page `Notification` bridge have both produced a
visible macOS banner. During the focused Teams Settings test, both page hooks
remained intact, permission was granted, and the page read that granted state,
but there were no window-constructor calls, page-registration calls, or shell
notification IPC calls. In-app alert activity increased instead.

This investigation inspected **public, unauthenticated Teams client assets**
from bootstrap build **26100212018**. An authenticated tenant can receive a
different build/configuration. Source-level findings below are not a claim that
an internal method's return value was captured in the live tenant session.
No message contents, authentication values, or full local-storage records were
printed or copied into this repository.

## Two separate paths

### Settings preview / in-app command

The settings component obtains the entity-commanding `toasts` service and calls
`toasts.showMessage` with the synthetic target `testNotification`.

Its prepared options contain a title, text, time, conversation reference,
quick-reply setting and image. The test honors Teams' message-preview setting:
when previews are disabled, Teams supplies its localized hidden-preview text.
It does **not directly call** the browser `Notification` API. The notifications
app also special-cases activation of `testNotification`.

This explains why the settings preview is not a sufficient test of the shell's
browser-to-native path. It may still enter additional notification processing;
we should not assume that every invocation is permanently in-app-only.

### Browser notifications

`NotificationsHandler.tryShowBrowserNotification` checks, in order:

1. Browser notification permission must be granted.
2. The user's out-of-tab notification preference must permit delivery.
3. If `suppressBrowserToastWhenAppIsFocused` is enabled and the main document has
   focus, return `skippedAppInFocus`.
4. Obtain the already-formatted notification payload and construct a browser
   `Notification`.

The current public production defaults enable this focus-suppression flag for
web platforms. Tenant/runtime settings can override defaults. The user's
notification booleans were checked read-only during this investigation; no
preference or feature flag was changed.

The existing shell hook is therefore at the correct boundary for this browser
path. There is no need to intercept WebSockets or read conversation databases
merely to receive its approved notifications.

## Lifecycle and payload

The browser-toast object exposes:

- `getWebToastOptions()` — the original prepared toast options.
- `getNotificationOptionsPayload()` — prepared title and browser options,
  including body, icon, timestamp and interaction/direction hints.
- `registerGetNotificationCallback()` — connects Teams' lifecycle to the
  notification object returned by the browser API.
- `getNotification()` and `closeNotification()` — lifecycle access/cleanup.

The handler attaches activation to the notification click and routes it through
`activateBrowserToast`. Closing a notification can dismiss its browser-toast
queue entry. Timeouts, in-app dismissal and higher-priority notifications can
close existing notifications. An adapter must preserve these semantics rather
than emit unrelated, untracked banners.

Notably, the inspected payload leaves `tag` undefined. Deduplication between
in-app and browser channels cannot assume Teams always supplies a browser tag.
A service-level logical identity, retained only in memory and never logged, is
preferable to matching private title/body strings.

## Implemented experimental foreground adapter

`src-tauri/src/teams-foreground-adapter.js` implements the guarded adapter below.
It is **off by default**; use the macOS **Help** menu checkbox **Also show native notifications
while focused (experimental)**. Only the opt-in boolean is persisted, and it can
be changed without restarting. `MSTEAMS_NATIVE_FOREGROUND=1` enables it for one
launch through `scripts/run-macos.sh` without changing the saved preference.

Mocked service/lifecycle tests and an integrated page-bridge test cover this
implementation. Live discovery in the tested tenant succeeded: `handler_found`
and `hooked` were true, and the settings test produced one browser candidate,
`skippedAppInFocus`, one adapted request and native acceptance. Inspect those
counters on other client versions; compatibility is not guaranteed.

1. Discover already-created services through known React provider references
   with bounded, descriptor-only lookup. Getters, arbitrary message properties,
   minified class names and numeric webpack module IDs are not used.
2. Observe `tryShowBrowserNotification` result categories. In trace mode only,
   also count `toasts.showMessage` calls and its known synthetic test target;
   never read or forward that command's message payload.
3. Keep the existing browser bridge for the `queued` path. Never emit a second
   native banner when that path has already delivered one.
4. Only when opted in, adapt compatible candidates returning
   `skippedAppInFocus`, using their prepared payload through the existing native
   bridge and wiring lifecycle/activation callbacks. Deduplicate pending/shown
   candidates by weak object identity. Failed native delivery restores the prior
   callback and original skip result; original exceptions/results are preserved.
5. Respect denied permission, disabled out-of-tab notifications, muted-message
   rules and preview redaction. Do not turn a `skippedNoPermissions` result into
   an OS notification. If a candidate never reaches the browser handler, an
   in-app-service adapter needs separate, guarded treatment, not a blanket
   conversion of all UI alerts.
6. If discovery/compatibility checks fail after a Teams update, leave Teams'
   behavior untouched and retain the standard browser hook.

Disabling restores the original method descriptors when they are still owned by
the adapter; it never overwrites a later application replacement. Discovery is
bounded to about a minute and can retry on focus or an opt-in toggle. Unsupported
or frozen service objects leave normal notifications unchanged.

This does not promise that every in-app-only settings preview becomes native:
if Teams never creates an approved browser candidate, it is intentionally not
promoted by guessing from rendered UI.

### Decouple in-app expiry from native presentation

The first live adapted test was accepted by macOS, but Teams closed its in-app
toast after about five seconds. The linked browser `close()` withdrew the native
request before the delayed foreground-presentation callback arrived. A successful
native acceptance therefore did not imply a visible banner.

For adapted foreground notifications only, a close after submission now emits
Teams' logical close event exactly once but leaves native dismissal to macOS.
Click/activation routing survives that logical close. The registry remains
bounded to 256 entries; evicted/stale notifications still bring the app forward
but cannot invoke a discarded page callback. Normal window/registration
notifications still obey browser close semantics. Closing before submission,
failed delivery, or adapter rollback uses forced native disposal, and old tagged
instances cannot delete a newer replacement. Regression tests simulate delayed
presentation and closure while acknowledgement is pending. The corrected live
Teams settings test was confirmed by the user: one focus-skipped candidate was
mirrored, accepted, logically closed and retained, then presented natively, with
no delivery errors. Native click-to-conversation and real background-message
coverage still require separate live validation.

Avoid global `document.hasFocus` spoofing, host/client-type spoofing,
`Object.prototype` assignment traps, feature-flag overrides and generic ARIA
alert scraping. Those approaches affect unrelated UI/presence behavior or can
turn non-notification status messages into banners.

## Latency observation

For the controlled page notification test, the trace measured roughly 6 ms from
request to native acceptance. The native foreground-presentation callback
arrived roughly 10.5 seconds later. The observed delay was after shell handoff;
its downstream cause has not been isolated. Watch close/dismiss timing in real
message traces before changing lifecycle semantics or claiming a latency fix.

## Public references

Inspected public Teams build assets:

- [Settings producer](https://teams.public.onecdn.static.microsoft/teams-modular-packages/hashed-assets/settings-app-481d7a5517cf78b8.js)
- [Core notification handler](https://teams.public.onecdn.static.microsoft/teams-modular-packages/hashed-assets/async-entry-b27cc96c3889ce4a.js)
- [Browser-toast queue and lifecycle](https://teams.public.onecdn.static.microsoft/teams-modular-packages/hashed-assets/async-apps-notifications-4a71fb2e11e7f837.js)
- [Production defaults](https://teams.public.onecdn.static.microsoft/teams-modular-packages/hashed-assets/config-prod-80421014aac5422c.js)

The established [teams-for-linux preload](https://github.com/IsmaelMartinez/teams-for-linux/blob/main/app/browser/preload.js)
also intercepts the standard `Notification` constructor. Its notification stub
and lifecycle handling are useful comparative evidence, not proof that every
Teams in-app event goes through that API. No third-party implementation was
copied into this shell.
