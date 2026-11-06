//! Unofficial Microsoft Teams desktop client built with Tauri.
//!
//! The whole application is a thin native shell around the Teams web client
//! (https://teams.microsoft.com). It loads Teams inside the system WebView and
//! adds the few behaviours that make the web app feel like a desktop app:
//!
//! * external links open in the user's default browser instead of hijacking the
//!   application window,
//! * in-app authentication popups (login.microsoftonline.com, ...) are opened in
//!   their own window so the OAuth flow completes inside the app,
//! * camera / microphone WebView requests are allowed, with OS-level permissions
//!   controlled by macOS; native notification authorization is requested separately,
//! * the WebView data directory is persisted so the user stays signed in,
//! * Teams' web notifications are bridged to native desktop notifications and
//!   drive a red unread-count badge on the system tray icon,
//! * closing the window hides the app to the tray instead of quitting.

// objc2::exception::catch cannot catch Objective-C exceptions with panic=abort.
// Wry relies on it when a WKURLSchemeTask is cancelled during navigation.
#[cfg(all(target_os = "macos", panic = "abort"))]
compile_error!("macOS WebKit requires panic=unwind for Objective-C exception handling");

#[cfg(target_os = "macos")]
mod app_export;
#[cfg(target_os = "macos")]
mod avatar_diagnostics;
mod configuration;
#[cfg(target_os = "macos")]
mod configurator;
#[cfg(target_os = "macos")]
mod dock_badge;
#[cfg(target_os = "macos")]
mod foreground_notifications;
#[cfg(target_os = "macos")]
mod media_permissions;
#[cfg(target_os = "macos")]
mod menu_bar;
#[cfg(target_os = "macos")]
mod native_notifications;
#[cfg(target_os = "macos")]
mod notification_trace;
#[cfg(target_os = "macos")]
mod screen_presence;
mod session_diagnostics;
#[cfg(target_os = "macos")]
mod session_restore;
#[cfg(target_os = "macos")]
mod zoom;

#[cfg(target_os = "macos")]
use foreground_notifications::ForegroundNotifications;
use session_diagnostics::snapshot;
#[cfg(target_os = "macos")]
use session_restore::SessionRestore;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::{NewWindowResponse, PermissionKind, PermissionResponse},
    AppHandle, Manager, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;

#[cfg(test)]
const TEAMS_URL: &str = "https://teams.cloud.microsoft/v2/";

// Deterministic fixture only; runtime profiles come from validated settings.
#[cfg(all(target_os = "macos", test))]
const WEBKIT_PROFILE_ID: [u8; 16] = [
    0xd0, 0xd7, 0xad, 0x45, 0x52, 0x8b, 0x41, 0x00, 0x93, 0xeb, 0x70, 0xb0, 0xc3, 0xbd, 0x13, 0xd0,
];

// Generic defaults are in config/default.json; custom hosts are private runtime settings.

/// Counter used to give every popup window a unique label.
static WINDOW_SEQ: AtomicU64 = AtomicU64::new(1);
static NOTIFICATION_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
enum WindowKind {
    Main,
    Popup,
}

/// Returns `true` when `url` may be loaded inside the application without
/// exposing the user to content outside Teams/Microsoft or the organization's
/// explicitly allowed HTTPS sign-in hosts.
fn is_trusted(url: &Url) -> bool {
    configuration::current().trusted(url)
}

/// Logs origins only: OAuth paths, query strings and fragments may hold secrets.
fn log_navigation(event: &str, url: &Url) {
    if std::env::var_os("MSTEAMS_DEBUG").is_some() {
        eprintln!(
            "[msteams] {event}: {} trusted={}",
            url.origin().ascii_serialization(),
            is_trusted(url)
        );
    }
}

/// Opens a URL in the user's default browser.
///
/// The opener plugin waits for the browser launcher to exit, which can take a
/// while, so it runs off the UI thread: on Linux this is called from inside a
/// WebKitGTK policy decision and blocking there would freeze the window.
fn open_external<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    log_navigation("external", url);
    let app = app.clone();
    let url = url.clone();
    std::thread::spawn(move || {
        use tauri_plugin_opener::OpenerExt;
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    });
}

/// Allow WebView requests needed for Teams. These responses do not grant macOS
/// camera/microphone or native notification authorization. Other requests keep
/// the browser default.
fn permission_response(kind: PermissionKind) -> PermissionResponse {
    if std::env::var_os("MSTEAMS_DEBUG").is_some() {
        let capture = match kind {
            PermissionKind::Camera => Some("camera"),
            PermissionKind::Microphone => Some("microphone"),
            _ => None,
        };
        if let Some(capture) = capture {
            eprintln!("[msteams] permission requested: {capture}");
        }
    }
    match kind {
        PermissionKind::Camera
        | PermissionKind::Microphone
        | PermissionKind::Notifications
        | PermissionKind::DisplayCapture
        | PermissionKind::MediaKeySystemAccess
        | PermissionKind::PointerLock
        | PermissionKind::Autoplay => PermissionResponse::Allow,
        _ => PermissionResponse::Default,
    }
}

/// Applies the WebKitGTK settings Teams needs, which the Linux WebView does not
/// enable by default:
///
/// * third-party cookies — WebKitGTK's default cookie accept policy is
///   `ExclusivelyFromMainDocumentDomain`, so cookies set by (or for) another
///   site are dropped. Teams uses third-party cookies during sign-in, and its
///   third-party-cookie probe (`report-third-party-cookies.html`, loaded in a
///   hidden iframe) fails without them, which makes Teams show that error page.
/// * media capabilities — the `MediaCapabilities` API used to negotiate codecs,
///   and `MediaStream` access to the camera and microphone.
/// * WebRTC — requests `RTCPeerConnection` support (`enable-webrtc`). This is
///   *not* enough on its own: the setting is a no-op unless the system WebKitGTK
///   was compiled with `ENABLE_WEB_RTC`. Ubuntu's `libwebkit2gtk-4.1-0` is built
///   without it, so `RTCPeerConnection` stays `undefined` and Teams reports
///   "You can't receive or make calls right now. Please try again by restarting
///   the app." The call is kept so the wrapper uses WebRTC on WebKitGTK builds
///   that do include it.
#[cfg(target_os = "linux")]
fn configure_webkit<R: Runtime>(window: &WebviewWindow<R>, url: Url) {
    use std::cell::Cell;
    use std::rc::Rc;

    use webkit2gtk::{
        CookieAcceptPolicy, CookieManagerExt, SettingsExt, WebContextExt, WebViewExt,
        WebsiteDataManagerExt,
    };

    let debug = std::env::var_os("MSTEAMS_DEBUG").is_some();
    let navigate_window = window.clone();

    let _ = window.with_webview(move |webview| {
        let view = webview.inner();

        if let Some(settings) = view.settings() {
            settings.set_enable_webrtc(true);
            settings.set_enable_media_stream(true);
            settings.set_enable_media_capabilities(true);

            // When MSTEAMS_DEBUG is set, mirror the page's console into stdout
            // and turn on the inspector so the wrapper can be debugged from a
            // terminal.
            if debug {
                settings.set_enable_write_console_messages_to_stdout(true);
                settings.set_enable_developer_extras(true);
            }
        }

        let manager = view.website_data_manager();
        if let Some(manager) = &manager {
            manager.set_itp_enabled(false);
            if debug {
                eprintln!("[msteams] ITP enabled: {}", manager.is_itp_enabled());
            }
        }

        // Both managers wrap the same network session, but setting the policy on
        // each of them keeps it applied whichever one the session was created
        // from. Note that disabling ITP resets the accept policy back to
        // `ExclusivelyFromMainDocumentDomain`, so the order below matters.
        let cookie_managers: Vec<_> = [
            manager.as_ref().and_then(|m| m.cookie_manager()),
            view.context().and_then(|c| c.cookie_manager()),
        ]
        .into_iter()
        .flatten()
        .collect();

        if cookie_managers.is_empty() {
            let _ = navigate_window.navigate(url);
            return;
        }

        // Setting the policy only queues a request to the network process. Wait
        // for the read-back before Teams can run its third-party-cookie probe.
        let pending = Rc::new(Cell::new(cookie_managers.len()));
        for cookies in cookie_managers {
            cookies.set_accept_policy(CookieAcceptPolicy::Always);

            let pending = Rc::clone(&pending);
            let window = navigate_window.clone();
            let url = url.clone();
            cookies.accept_policy(None::<&gio::Cancellable>, move |policy| {
                if debug {
                    eprintln!("[msteams] cookie accept policy: {policy:?}");
                }
                pending.set(pending.get() - 1);
                if pending.get() == 0 {
                    let _ = window.navigate(url);
                }
            });
        }
    });
}

/// Installs the "external links open in the system browser" policy directly on
/// the WebKitGTK webview.
///
/// Tauri's `on_navigation` is a thin wrapper around WebKitGTK's
/// `decide-policy` signal, which is emitted for **every** frame — including
/// sub-frames. Teams loads part of itself in hidden iframes (the
/// third-party-cookie probe, widgets, …); blocking those because their host is
/// not in `TRUSTED_HOSTS` hijacked them into the user's browser and made Teams
/// believe the browser was broken.
///
/// WebKitGTK does not expose whether a navigation targets the main frame, so
/// the policy is only applied where it cannot break the page: user-initiated
/// navigations (link clicks and form submissions) to hosts outside
/// `TRUSTED_HOSTS` are opened in the browser, everything else — sub-frames,
/// redirects and script-driven loads — is left alone.
#[cfg(target_os = "linux")]
fn install_navigation_policy<R: Runtime>(window: &WebviewWindow<R>, app: AppHandle<R>) {
    use webkit2gtk::{
        glib::Cast, NavigationPolicyDecision, NavigationPolicyDecisionExt, PolicyDecisionExt,
        PolicyDecisionType, URIRequestExt, WebViewExt,
    };

    let _ = window.with_webview(move |webview| {
        let view = webview.inner();

        view.connect_decide_policy(move |_view, decision, decision_type| {
            if decision_type != PolicyDecisionType::NavigationAction {
                return false;
            }

            let Some(action) = decision
                .clone()
                .downcast::<NavigationPolicyDecision>()
                .ok()
                .and_then(|decision| decision.navigation_action())
            else {
                return false;
            };

            let Some(url) = action
                .request()
                .and_then(|request| request.uri())
                .and_then(|uri| uri.parse::<Url>().ok())
            else {
                return false;
            };

            log_navigation("navigation", &url);
            if is_trusted(&url) || !action.is_user_gesture() {
                return false;
            }

            open_external(&app, &url);
            decision.ignore();
            true
        });
    });
}

/// Builds a WebView window that loads `url` and shares the common Teams
/// behaviours (external link handling, popup handling and permissions).
fn create_window<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    url: Url,
    kind: WindowKind,
    data_dir: PathBuf,
) -> tauri::Result<WebviewWindow<R>> {
    let nav_app = app.clone();
    let popup_app = app.clone();
    let popup_dir = data_dir.clone();

    // Linux and popups start on about:blank; macOS session restore uses a local
    // embedded data loading page (no asynchronous custom-protocol task). Teams
    // navigation starts only once native preparation is in
    // place *before* the first request goes out. Teams runs its third-party
    // cookie probe right at start-up and treats a blocked probe as "this
    // browser is broken".
    let blank = Url::parse("about:blank").expect("about:blank is a valid URL");
    let initial_url = WebviewUrl::External(blank);
    #[cfg(target_os = "macos")]
    let initial_url = if matches!(kind, WindowKind::Main) && app.state::<SessionRestore>().enabled()
    {
        WebviewUrl::External(session_restore::restore_page_url())
    } else {
        initial_url
    };
    let builder = WebviewWindowBuilder::new(app, label, initial_url)
        .title(configuration::current().name.clone())
        // Set at the WebView level before navigation, for both HTTP requests
        // and navigator.userAgent, including in authentication popups.
        .user_agent(configuration::current().browser_user_agent())
        .data_directory(data_dir)
        .min_inner_size(400.0, 300.0)
        .on_page_load(|window, payload| {
            log_navigation("page-load", payload.url());
            #[cfg(target_os = "macos")]
            if window.label() == "main" {
                if is_notification_origin(payload.url()) {
                    if let Some(zoom) = window.app_handle().try_state::<zoom::Zoom>() {
                        zoom.restore(&window);
                    }
                } else {
                    let _ = window.set_zoom(1.0);
                }
            }
            if std::env::var_os("MSTEAMS_DEBUG").is_some()
                && matches!(
                    payload.url().host_str(),
                    Some("teams.cloud.microsoft" | "teams.microsoft.com")
                )
            {
                let _ = window.eval(MEDIA_DIAGNOSTICS_JS);
                let window = window.clone();
                std::thread::spawn(move || {
                    // Device enumeration is asynchronous. Read only the sanitized
                    // summary, never device labels, identifiers or page content.
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let _ = window
                        .eval_with_callback("window.__msteamsMediaDiagnostics || null", |result| {
                            eprintln!("[msteams] media capabilities: {result}")
                        });
                });
            }
        });

    // Tauri's native file-drop handler consumes macOS drops before WKWebView
    // receives them. Let Teams handle user-selected File/DataTransfer objects
    // through HTML5; no native paths, filesystem grant or custom upload bridge.
    let builder = if matches!(kind, WindowKind::Main) {
        builder.disable_drag_drop_handler()
    } else {
        builder
    };

    // All windows must share the same persistent profile during SSO. Older
    // macOS versions fall back to WebKit's default persistent store.
    #[cfg(target_os = "macos")]
    let builder = builder.data_store_identifier(configuration::current().profile());

    // Notification/capture hooks belong only to the main window, not auth popups.
    let builder = if matches!(kind, WindowKind::Main) {
        #[cfg(target_os = "macos")]
        let builder = if notification_trace::enabled() {
            builder.initialization_script("window.__msteamsNotificationTraceEnabled = true;")
        } else {
            builder
        };
        let builder = builder
            .initialization_script(format!(
                "window.__msteamsAllowedNotificationOrigins = {};",
                serde_json::to_string(&configuration::current().notification_origins)
                    .expect("origins serialize")
            ))
            .initialization_script(INJECT_JS);
        #[cfg(target_os = "macos")]
        let builder = builder
            .initialization_script("window.__msteamsHasNativeMediaStatus = true;")
            .initialization_script(include_str!("teams-foreground-adapter.js"));
        let builder = builder.initialization_script(include_str!("media-permissions.js"));
        #[cfg(target_os = "macos")]
        let builder = builder
            .initialization_script(include_str!("zoom-hotkeys.js"))
            .initialization_script(include_str!("screen-presence.js"));
        builder
    } else {
        builder
    };

    // Keep Teams/Microsoft navigation in-app, send the rest to the browser. On
    // Linux the policy is installed on the WebKitGTK webview instead, because
    // Tauri's handler also sees (and would block) sub-frame navigations.
    #[cfg(not(target_os = "linux"))]
    let builder = builder.on_navigation(move |url| {
        log_navigation("navigation", url);
        #[cfg(target_os = "macos")]
        if SessionRestore::is_continue_url(url) {
            if matches!(kind, WindowKind::Main) {
                let state = nav_app.state::<SessionRestore>().inner().clone();
                // Leave WebKit's navigation-policy callback before navigating.
                std::thread::spawn(move || {
                    state.continue_without_restoring();
                });
            }
            return false;
        }
        if is_trusted(url) {
            true
        } else {
            open_external(&nav_app, url);
            false
        }
    });

    let builder = builder
        // `window.open` / target="_blank": complete OAuth popups in-app, open
        // ordinary web links in the system browser.
        .on_new_window(move |url, _features| {
            log_navigation("popup", &url);
            if is_trusted(&url) {
                let label = format!("auth-{}", WINDOW_SEQ.fetch_add(1, Ordering::Relaxed));
                match create_window(
                    &popup_app,
                    &label,
                    url,
                    WindowKind::Popup,
                    popup_dir.clone(),
                ) {
                    Ok(window) => NewWindowResponse::Create { window },
                    Err(_) => NewWindowResponse::Deny,
                }
            } else {
                open_external(&popup_app, &url);
                NewWindowResponse::Deny
            }
        })
        .on_permission_request(|_webview, kind| permission_response(kind));

    let window = match kind {
        WindowKind::Main => builder.inner_size(1280.0, 820.0).center().build(),
        WindowKind::Popup => builder.inner_size(520.0, 720.0).center().build(),
    }?;

    // Teams needs third-party cookies and WebRTC; WebKitGTK disables both by
    // default. Navigation starts only after the cookie policy is confirmed.
    #[cfg(target_os = "linux")]
    {
        configure_webkit(&window, url);
        install_navigation_policy(&window, nav_app);
    }

    // Never gate navigation on a JavaScript callback. Opt-in macOS restore uses
    // native APIs on a worker. An embedded loading page allows Keychain approval or
    // an explicit skip, with a bounded two-minute fallback.
    #[cfg(target_os = "macos")]
    if matches!(kind, WindowKind::Main) {
        app.state::<SessionRestore>().navigate(&window, url);
    } else {
        window.navigate(url)?;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    window.navigate(url)?;

    Ok(window)
}

/// Passive capability probe: no media capture, permission requests or network calls.
/// Device enumeration records counts only, never labels, IDs or group IDs.
const MEDIA_DIAGNOSTICS_JS: &str = r#"
(function () {
  var result = {
    secureContext: window.isSecureContext === true,
    peerConnection: typeof RTCPeerConnection === 'function',
    getUserMedia: !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia),
    displayCapture: !!(navigator.mediaDevices && navigator.mediaDevices.getDisplayMedia),
    mediaCapabilities: typeof navigator.mediaCapabilities !== 'undefined',
    audioContext: typeof AudioContext === 'function',
    enumerateDevices: !!(navigator.mediaDevices && navigator.mediaDevices.enumerateDevices),
    setSinkId: typeof HTMLMediaElement.prototype.setSinkId === 'function',
    selectAudioOutput: !!(navigator.mediaDevices && navigator.mediaDevices.selectAudioOutput),
    capturePermissionResults: window.__msteamsMediaPermissionResults || null
  };
  window.__msteamsMediaDiagnostics = result;
  if (result.enumerateDevices) {
    result.devices = 'pending';
    navigator.mediaDevices.enumerateDevices().then(function (devices) {
      var counts = {audioinput: 0, audiooutput: 0, videoinput: 0};
      devices.forEach(function (device) {
        if (Object.prototype.hasOwnProperty.call(counts, device.kind)) counts[device.kind]++;
      });
      result.devices = counts;
    }).catch(function (e) { result.devices = e.name; });
  }
  try {
    var peer = new RTCPeerConnection({iceServers: []});
    peer.close();
    result.peerConstruction = 'ok';
  } catch (e) {
    result.peerConstruction = e.name;
  }
  return result;
})();
"#;

/// JavaScript injected into every Teams window before the page scripts run.
///
/// It bridges the two signals Teams gives us about unread activity to the Rust
/// side: (1) the amount of web notifications fired (`new Notification(...)`),
/// which is the most reliable in-app signal, and (2) the unread counter Teams
/// puts in `document.title` (e.g. `(3) Microsoft Teams...`). Whichever is
/// higher wins, and the counter resets when the user looks at the window.
#[cfg(target_os = "macos")]
const INJECT_JS: &str = include_str!("notification-bridge.js");

#[cfg(not(target_os = "macos"))]
const INJECT_JS: &str = r#"
(function () {
  if (window.__msteamsHooked) return;
  window.__msteamsHooked = true;

  var sessionCount = 0;
  var titleCount = 0;
  var last = -1;

  function current() { return Math.max(sessionCount, titleCount); }

  function report() {
    var c = current();
    if (c === last) return;
    last = c;
    try {
      window.__TAURI_INTERNALS__.invoke("set_unread_count", { count: c });
    } catch (e) {}
  }

  function notify(title, body) {
    try {
      window.__TAURI_INTERNALS__.invoke("show_notification", {
        title: String(title == null ? "Microsoft Teams" : title),
        body: String(body == null ? "" : body)
      });
    } catch (e) {}
  }

  try {
    function TmsNotification(title, options) {
      options = options || {};
      sessionCount += 1;
      report();
      notify(title, options.body);
      this.title = title;
      this.body = options.body || "";
      this.onclick = null;
      this.close = function () {};
      this.addEventListener = function () {};
      this.removeEventListener = function () {};
      this.dispatchEvent = function () { return true; };
    }
    TmsNotification.permission = "granted";
    TmsNotification.requestPermission = function (cb) {
      if (typeof cb === "function") cb("granted");
      return Promise.resolve("granted");
    };
    window.Notification = TmsNotification;
  } catch (e) {}

  function scanTitle() {
    var m = /^\(\s*(\d+)\s*\)/.exec(document.title || "");
    var n = m ? parseInt(m[1], 10) : 0;
    if (n !== titleCount) { titleCount = n; report(); }
  }

  try {
    var head = document.head || document.documentElement;
    new MutationObserver(scanTitle).observe(head, {
      subtree: true, childList: true, characterData: true
    });
  } catch (e) {}

  window.addEventListener("focus", function () { sessionCount = 0; report(); });
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden) { sessionCount = 0; report(); }
  });
  setInterval(scanTitle, 3000);
  setTimeout(scanTitle, 1500);
})();
"#;

/// Shared unread-message count driving the tray badge.
struct Unread(AtomicU32);

/// Minimal 3x5 bitmap glyphs for `0`-`9` and `+` (row-major, MSB = left).
const GLYPHS: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
    [0b000, 0b010, 0b111, 0b010, 0b000], // +
];

fn glyph(c: u8) -> Option<&'static [u8; 5]> {
    match c {
        b'0'..=b'9' => Some(&GLYPHS[(c - b'0') as usize]),
        b'+' => Some(&GLYPHS[10]),
        _ => None,
    }
}

#[inline]
fn put_pixel(buf: &mut [u8], w: usize, h: usize, x: i32, y: i32, px: [u8; 4]) {
    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return;
    }
    let i = (y as usize * w + x as usize) * 4;
    buf[i..i + 4].copy_from_slice(&px);
}

/// Composites a red circle with a white count onto a copy of the base icon.
fn render_badge(base: &Image<'_>, count: u32) -> Image<'static> {
    let w = base.width() as usize;
    let h = base.height() as usize;
    let mut buf = base.rgba().to_vec();

    if count > 0 {
        let label = if count > 99 {
            "99+".to_string()
        } else {
            count.to_string()
        };

        let r = (w as i32 / 4).max(5);
        let cx = w as i32 - r - 1;
        let cy = r + 1;
        for y in (cy - r)..=(cy + r) {
            for x in (cx - r)..=(cx + r) {
                let (dx, dy) = (x - cx, y - cy);
                if dx * dx + dy * dy <= r * r {
                    put_pixel(&mut buf, w, h, x, y, [214, 40, 40, 255]);
                }
            }
        }

        let chars: Vec<u8> = label.into_bytes();
        let n = chars.len() as i32;
        let (gw, gh, sp) = (3, 5, 1);
        let text_w = n * gw + (n - 1) * sp;
        let max_dim = (2 * r - 4).max(1);
        let scale = (max_dim / text_w.max(1)).min(max_dim / gh).max(1);
        let start_x = cx - text_w * scale / 2;
        let start_y = cy - gh * scale / 2;

        for (i, ch) in chars.iter().enumerate() {
            let Some(g) = glyph(*ch) else { continue };
            let gx = start_x + i as i32 * (gw + sp) * scale;
            for (row, bits) in g.iter().enumerate() {
                for col in 0..gw {
                    if (bits >> (gw - 1 - col)) & 1 == 0 {
                        continue;
                    }
                    for sy in 0..scale {
                        for sx in 0..scale {
                            put_pixel(
                                &mut buf,
                                w,
                                h,
                                gx + col * scale + sx,
                                start_y + row as i32 * scale + sy,
                                [255, 255, 255, 255],
                            );
                        }
                    }
                }
            }
        }
    }

    Image::new_owned(buf, w as u32, h as u32)
}

/// Re-renders the tray icon and tooltip for the given unread count.
fn update_tray<R: Runtime>(app: &AppHandle<R>, count: u32) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    if let Some(base) = app.default_window_icon() {
        let _ = tray.set_icon(Some(render_badge(base, count)));
    }
    let tip = if count == 0 {
        "Microsoft Teams".to_string()
    } else {
        format!("Microsoft Teams — {count} unread")
    };
    let _ = tray.set_tooltip(Some(tip));
}

#[cfg(target_os = "macos")]
fn reload_main(app: &AppHandle) {
    show_main(app);
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let current = window.url().ok();
    // Do not interrupt an active Keychain restore or consume its snapshot twice.
    if current
        .as_ref()
        .is_some_and(SessionRestore::is_restore_page)
    {
        return;
    }
    let result = if current.as_ref().is_some_and(is_notification_origin) {
        window.reload()
    } else {
        // A post-login blank document needs the configured start page, not
        // another reload of about:blank. No cookie/profile data is cleared.
        window
            .navigate(Url::parse(&configuration::current().teams_url).expect("validated start URL"))
    };
    if result.is_err() {
        eprintln!("[msteams] could not reload Teams");
    }
}

fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Closes the authentication popups (`auth-*`).
///
/// Teams closes those windows itself once the OAuth flow finishes, but the page
/// can only destroy its own web view (`window.close()`), which leaves the window
/// shell behind as an empty frame. Closing them when the user comes back to the
/// main window keeps sign-in from littering the desktop.
fn close_popups<R: Runtime>(app: &AppHandle<R>) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("auth-") {
            let _ = window.destroy();
        }
    }
}

/// Called from the injected script whenever the unread count changes.
#[tauri::command]
fn set_unread_count(app: AppHandle, unread: tauri::State<'_, Unread>, count: u32) {
    unread.0.store(count, Ordering::Relaxed);
    update_tray(&app, count);
    #[cfg(target_os = "macos")]
    dock_badge::update(&app, count);
}

/// Called from the injected script to show a native notification for a web
/// notification fired by Teams.
fn is_notification_origin(url: &Url) -> bool {
    configuration::current().notification_origin(url)
}

fn require_notification_window(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" && window.url().is_ok_and(|url| is_notification_origin(&url)) {
        Ok(())
    } else {
        Err("Notification access is limited to the main Teams page".into())
    }
}

fn notification_payload(
    title: String,
    body: String,
    id: Option<String>,
) -> Result<(String, String, String), String> {
    let id =
        id.unwrap_or_else(|| format!("shell-{}", NOTIFICATION_SEQ.fetch_add(1, Ordering::Relaxed)));
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err("Invalid notification identifier".into());
    }
    let title = if title.trim().is_empty() {
        "Microsoft Teams".to_owned()
    } else {
        title.chars().take(512).collect()
    };
    Ok((id, title, body.chars().take(4096).collect()))
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn screen_presence_status(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<screen_presence::Snapshot, String> {
    require_notification_window(&window)?;
    Ok(app.state::<screen_presence::ScreenPresence>().snapshot())
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn media_permission_status(window: WebviewWindow) -> Result<media_permissions::Status, String> {
    require_notification_window(&window)?;
    Ok(media_permissions::status())
}

#[tauri::command]
fn zoom_teams(app: AppHandle, window: WebviewWindow, action: String) -> Result<(), String> {
    require_notification_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        let action = zoom::Action::parse(&action).ok_or("Invalid zoom action")?;
        app.state::<zoom::Zoom>().apply(&window, action)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, action);
        Err("Native zoom controls are macOS-only".into())
    }
}

#[tauri::command]
fn foreground_notifications_enabled(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    require_notification_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        Ok(app.state::<ForegroundNotifications>().enabled())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(false)
    }
}

#[tauri::command]
async fn notification_permission(
    app: AppHandle,
    window: WebviewWindow,
    request: bool,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    notification_trace::event(if request {
        "ipc_permission_request"
    } else {
        "ipc_permission_query"
    });
    require_notification_window(&window)?;
    #[cfg(target_os = "macos")]
    {
        let _ = app.state::<native_notifications::NativeNotifications>();
        tauri::async_runtime::spawn_blocking(move || {
            native_notifications::permission(request).map(str::to_owned)
        })
        .await
        .map_err(|_| "Notification permission operation failed".to_owned())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::plugin::PermissionState;
        let state = if request {
            app.notification().request_permission()
        } else {
            app.notification().permission_state()
        }
        .map_err(|_| "Notification permission operation failed".to_owned())?;
        Ok(match state {
            PermissionState::Granted => "granted",
            PermissionState::Denied => "denied",
            _ => "default",
        }
        .into())
    }
}

#[tauri::command]
async fn show_notification(
    app: AppHandle,
    window: WebviewWindow,
    title: String,
    body: String,
    id: Option<String>,
    silent: Option<bool>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    notification_trace::event("ipc_show_received");
    require_notification_window(&window).inspect_err(|_| {
        #[cfg(target_os = "macos")]
        notification_trace::event("ipc_origin_rejected");
    })?;
    let (id, title, body) = notification_payload(title, body, id).inspect_err(|_| {
        #[cfg(target_os = "macos")]
        notification_trace::event("ipc_payload_rejected");
    })?;
    #[cfg(target_os = "macos")]
    {
        let _ = app.state::<native_notifications::NativeNotifications>();
        tauri::async_runtime::spawn_blocking(move || {
            native_notifications::show(&id, &title, &body, silent.unwrap_or(false))
        })
        .await
        .map_err(|_| "Notification delivery operation failed".to_owned())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, silent);
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|_| "Notification delivery operation failed".into())
    }
}

#[tauri::command]
async fn close_notification(window: WebviewWindow, id: String) -> Result<(), String> {
    require_notification_window(&window)?;
    let (id, _, _) = notification_payload(String::new(), String::new(), Some(id))?;
    #[cfg(target_os = "macos")]
    tauri::async_runtime::spawn_blocking(move || native_notifications::close(&id))
        .await
        .map_err(|_| "Notification close operation failed".to_owned())?;
    #[cfg(not(target_os = "macos"))]
    let _ = id;
    Ok(())
}

#[cfg(target_os = "macos")]
fn send_test_notification(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let result = native_notifications::permission(true).and_then(|permission| {
            if permission != "granted" {
                return Err("Notifications are disabled. Enable Microsoft Teams in System Settings → Notifications.".into());
            }
            let id = format!("test-{}", NOTIFICATION_SEQ.fetch_add(1, Ordering::Relaxed));
            native_notifications::show(&id, "Microsoft Teams", "Native notification bridge is working. Click to open Teams.", false)
        });
        match result {
            Ok(()) => eprintln!("[msteams] test notification accepted: true"),
            Err(error) => {
                eprintln!("[msteams] test notification failed: {error}");
                let ui_handle = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    show_main(&ui_handle);
                    native_notifications::show_test_error(&error);
                });
            }
        }
    });
}

/// Application entry point shared by the desktop binary and (potentially)
/// mobile targets.
pub fn run() {
    let context = tauri::generate_context!();
    #[cfg(target_os = "macos")]
    let (context, launch) = {
        let mut context = context;
        let launch = configurator::prepare(&mut context);
        (context, launch)
    };
    #[cfg(not(target_os = "macos"))]
    let _ = configuration::initialize(configuration::AppConfig::default());
    #[cfg(target_os = "macos")]
    if std::env::args().any(|arg| arg == "--check-config") {
        println!(
            "{}",
            serde_json::json!({"valid": launch.error.is_none(),
            "mode": if launch.configurator { "configurator" } else { "runtime" },
            "identifier": context.config().identifier,
            "profile_id": if launch.configurator { None } else { configuration::current().profile_id.as_ref() },
            "error": launch.error})
        );
        std::process::exit(if launch.error.is_some() { 1 } else { 0 });
    }
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_notification::init());
    let app = builder
        .invoke_handler(tauri::generate_handler![
            set_unread_count,
            show_notification,
            notification_permission,
            close_notification,
            foreground_notifications_enabled,
            zoom_teams,
            #[cfg(target_os = "macos")]
            media_permission_status,
            #[cfg(target_os = "macos")]
            screen_presence_status,
            #[cfg(target_os = "macos")]
            configurator::configurator_status,
            #[cfg(target_os = "macos")]
            configurator::configurator_open_file,
            #[cfg(target_os = "macos")]
            configurator::configurator_export,
            #[cfg(target_os = "macos")]
            configurator::configurator_open_generated
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            if launch.configurator { configurator::setup(app, launch.error)?; return Ok(()); }
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let capability = serde_json::json!({
                "identifier": "configured-notifications",
                "windows": ["main"],
                "local": false,
                "remote": {"urls": configuration::current().notification_origins.iter().map(|origin| format!("{origin}/*")).collect::<Vec<_>>()},
                "permissions": ["allow-show-notification", "allow-notification-permission", "allow-close-notification", "allow-foreground-notifications-enabled", "allow-set-unread-count", "allow-zoom-teams", "allow-media-permission-status", "allow-screen-presence-status"]
            });
            app.add_capability(capability.to_string().as_str())?;

            app.manage(Unread(AtomicU32::new(0)));
            #[cfg(target_os = "macos")]
            app.manage(ForegroundNotifications::new(data_dir.clone()));
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                app.manage(native_notifications::NativeNotifications::new(move |id| {
                    show_main(&handle);
                    if let Some(window) = handle.get_webview_window("main") {
                        if let Ok(id) = serde_json::to_string(&id) {
                            let _ = window.eval(format!("if (window.__msteamsNotificationClick) window.__msteamsNotificationClick({id});"));
                        }
                    }
                }));
            }
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                app.manage(screen_presence::ScreenPresence::new(data_dir.clone(), move |_event| {
                    let dispatch = handle.clone();
                    let _ = handle.run_on_main_thread(move || {
                        // Read the latest state at delivery, not a notification's
                        // earlier snapshot: a queued event must not undo opt-out.
                        let Some(state) = dispatch.try_state::<screen_presence::ScreenPresence>().map(|state| state.snapshot()) else { return; };
                        if let Some(window) = dispatch.get_webview_window("main") {
                            if window.url().is_ok_and(|url| is_notification_origin(&url)) {
                                if let Ok(json) = serde_json::to_string(&state) {
                                    let _ = window.eval(format!("if(window.__msteamsScreenPresence)window.__msteamsScreenPresence.receive({json});"));
                                }
                            }
                        }
                    });
                }));
            }
            #[cfg(target_os = "macos")]
            app.manage(SessionRestore::new(data_dir.clone(), configuration::current().profile()));
            #[cfg(target_os = "macos")]
            session_restore::install_quit_menu(app.handle())?;
            #[cfg(target_os = "macos")]
            zoom::install(app.handle(), data_dir.clone())?;
            #[cfg(target_os = "macos")]
            dock_badge::update(app.handle(), 0);

            // System tray: click to restore the window, right-click for the menu.
            let show_i =
                MenuItem::with_id(app, "show", format!("Show {}", configuration::current().name), true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "Hide Window", true, None::<&str>)?;
            let media_i = MenuItem::with_id(
                app,
                "media-permissions",
                "Enable microphone and camera",
                true,
                None::<&str>,
            )?;
            let notification_i = MenuItem::with_id(
                app, "test-notification", "Send test notification", true, None::<&str>,
            )?;
            let session_i = MenuItem::with_id(
                app,
                "session-diagnostics",
                "Save session diagnostics (metadata only)",
                true,
                None::<&str>,
            )?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&show_i, &hide_i, &media_i, &notification_i, &session_i, &separator, &quit_i],
            )?;

            #[cfg(target_os = "macos")]
            {
                let item = MenuItem::with_id(app, "avatar-diagnostics", "Save avatar diagnostics (metadata only)", true, None::<&str>)?;
                menu.insert(&item, 5)?;
            }

            #[cfg(target_os = "macos")]
            if notification_trace::enabled() {
                let web_test = MenuItem::with_id(app, "test-web-notification", "Send web-bridge test notification", true, None::<&str>)?;
                menu.insert(&web_test, 4)?;
            }

            #[cfg(target_os = "macos")]
            let restore_i = {
                let item = tauri::menu::CheckMenuItem::with_id(
                    app,
                    "restore-session",
                    "Restore session on restart (Keychain)",
                    true,
                    app.state::<SessionRestore>().enabled(),
                    None::<&str>,
                )?;
                let forget = MenuItem::with_id(
                    app,
                    "forget-restored-session",
                    "Disable session restore and forget saved session",
                    true,
                    None::<&str>,
                )?;
                menu.insert_items(&[&item, &forget], 4)?;
                item
            };

            #[cfg(target_os = "macos")]
            let foreground_i = {
                let item = tauri::menu::CheckMenuItem::with_id(
                    app, "foreground-native", "Also show native notifications while focused (experimental)",
                    true, app.state::<ForegroundNotifications>().enabled(), None::<&str>,
                )?;
                menu.insert(&item, 4)?;
                item
            };

            #[cfg(target_os = "macos")]
            let screen_presence_i = {
                let item = tauri::menu::CheckMenuItem::with_id(app, "screen-presence", "Keep online until screen saver / lock (experimental)", true,
                    app.state::<screen_presence::ScreenPresence>().enabled(), None::<&str>)?;
                let status = MenuItem::with_id(app, "screen-presence-diagnostics", "Presence prototype status", true, None::<&str>)?;
                menu.insert_items(&[&item, &status], 4)?;
                item
            };
            #[cfg(target_os = "macos")]
            let tray_visibility_i = menu_bar::install(app.handle(), &menu, data_dir.clone())?;
            #[cfg(target_os = "macos")]
            let tray_visibility_event_i = tray_visibility_i.clone();

            // Application-wide dispatch: Help commands remain usable even when
            // the tray icon is hidden. Do not bind these actions to tray events.
            app.on_menu_event(move |app, event| match event.id.as_ref() {
                    #[cfg(target_os = "macos")]
                    menu_bar::RELOAD_ID => reload_main(app),
                    #[cfg(target_os = "macos")]
                    menu_bar::TRAY_ID => {
                        let state = app.state::<menu_bar::TrayVisibility>();
                        let visible = tray_visibility_event_i.is_checked().unwrap_or(state.visible());
                        if state.set_visible(visible, |visible| {
                            app.tray_by_id("main").ok_or("Tray icon unavailable")?
                                .set_visible(visible).map_err(|_| "Could not change tray visibility".into())
                        }).is_err() {
                            let _ = tray_visibility_event_i.set_checked(state.visible());
                            eprintln!("[msteams] could not change tray visibility");
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "screen-presence" => {
                        let state = app.state::<screen_presence::ScreenPresence>();
                        let enabled = screen_presence_i.is_checked().unwrap_or(state.enabled());
                        if state.set_enabled(enabled).is_err() {
                            let _ = screen_presence_i.set_checked(state.enabled());
                            eprintln!("[msteams] could not enable native screen presence prototype");
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "screen-presence-diagnostics" => {
                        if let Some(window) = app.get_webview_window("main") {
                            if window.url().is_ok_and(|url| is_notification_origin(&url)) {
                                screen_presence::show_diagnostics(&window);
                            }
                        }
                    }
                    "show" => show_main(app),
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "media-permissions" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                            let _ = window.eval(
                                "if (window.__msteamsRequestMediaPermissions) { window.__msteamsRequestMediaPermissions(); }",
                            );
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "test-web-notification" => {
                        notification_trace::event("web_test_requested");
                        if let Some(window) = app.get_webview_window("main") {
                            if window.url().is_ok_and(|url| is_notification_origin(&url)) {
                                let _ = window.eval_with_callback(
                                    "(function () { try { new window.Notification('Teams web-bridge test', {body: 'Synthetic page notification through the same bridge as Teams.', silent: false}); return true; } catch (_) { return false; } })();",
                                    |raw| notification_trace::event(if serde_json::from_str::<bool>(&raw).unwrap_or(false) { "web_test_constructed" } else { "web_test_constructor_failed" }),
                                );
                            } else { notification_trace::event("web_test_not_on_teams"); }
                        }
                    }
                    "test-notification" => {
                        #[cfg(target_os = "macos")]
                        send_test_notification(app);
                        #[cfg(not(target_os = "macos"))]
                        let _ = app.notification().builder().title("Microsoft Teams")
                            .body("Native notification bridge is working.").show();
                    }
                    #[cfg(target_os = "macos")]
                    "avatar-diagnostics" => {
                        if let Some(window) = app.get_webview_window("main") {
                            avatar_diagnostics::snapshot(&window);
                        }
                    }
                    "session-diagnostics" => {
                        if let Some(window) = app.get_webview_window("main") {
                            snapshot(&window, "manual");
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "foreground-native" => {
                        let state = app.state::<ForegroundNotifications>();
                        let enabled = foreground_i.is_checked().unwrap_or(false);
                        if state.set_enabled(enabled).is_err() {
                            let _ = foreground_i.set_checked(state.enabled());
                            eprintln!("[msteams] could not save foreground notification preference");
                        } else if let Some(window) = app.get_webview_window("main") {
                            let _ = window.eval(format!("if (window.__msteamsForegroundAdapter) window.__msteamsForegroundAdapter.setEnabled({enabled});"));
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "restore-session" => {
                        let state = app.state::<SessionRestore>();
                        let enabled = restore_i.is_checked().unwrap_or(false);
                        if state.set_enabled(enabled).is_err() {
                            let _ = restore_i.set_checked(state.enabled());
                            eprintln!("[msteams] could not save session-restore preference");
                        }
                    }
                    #[cfg(target_os = "macos")]
                    "forget-restored-session" => {
                        let state = app.state::<SessionRestore>();
                        let _ = restore_i.set_checked(false);
                        if state.set_enabled(false).is_err() {
                            eprintln!("[msteams] could not persist disabled session-restore preference");
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });

            let tray = TrayIconBuilder::with_id("main")
                .show_menu_on_left_click(false)
                .tooltip(configuration::current().name.clone())
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                });
            // On macOS commands live in Help; the status item only restores
            // the window. Other platforms keep the existing tray context menu.
            #[cfg(not(target_os = "macos"))]
            let tray = tray.menu(&menu);
            let mut tray = tray;
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            let tray = tray.build(app)?;
            #[cfg(target_os = "macos")]
            if tray.set_visible(app.state::<menu_bar::TrayVisibility>().visible()).is_err() {
                // Optional icon visibility must not prevent startup.
                let _ = tray.set_visible(true);
                app.state::<menu_bar::TrayVisibility>().show_fallback();
                let _ = tray_visibility_i.set_checked(true);
                eprintln!("[msteams] using visible tray icon after visibility failure");
            }
            #[cfg(not(target_os = "macos"))]
            let _ = tray;

            let url = Url::parse(&configuration::current().teams_url).expect("validated start URL");
            let main = create_window(app.handle(), "main", url, WindowKind::Main, data_dir)?;

            #[cfg(target_os = "macos")]
            if std::env::var("MSTEAMS_TEST_NOTIFICATION").as_deref() == Ok("1") {
                send_test_notification(app.handle());
            }
            #[cfg(target_os = "macos")]
            notification_trace::start(app.handle());

            // Teams closes its sign-in popups from JavaScript, which only
            // destroys their web view; drop the leftover windows once the user is
            // back in the main window.
            let handle = app.handle().clone();
            main.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Focused(true)) {
                    close_popups(&handle);
                }
            });

            Ok(())
        })
        // Closing the main window hides the app to the tray; quit from the menu.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                } else if window.label() == "configurator" {
                    window.app_handle().exit(0);
                }
            }
        })
        .build(context)
        .expect("error while building the Microsoft Teams desktop application");

    app.run(|_app, event| {
        #[cfg(target_os = "macos")]
        match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if let Some(state) = _app.try_state::<SessionRestore>() {
                    state.on_exit(_app, &api, code.unwrap_or(0));
                }
            }
            tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => {
                // A hidden status item must never strand a closed/hidden app.
                if _app.get_webview_window("main").is_some() {
                    show_main(_app);
                } else if let Some(window) = _app.get_webview_window("configurator") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        }
        #[cfg(not(target_os = "macos"))]
        let _ = event;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(size: u32) -> Image<'static> {
        Image::new_owned(vec![0u8; (size * size * 4) as usize], size, size)
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn notification_bridge_accepts_only_exact_https_teams_origins() {
        assert!(is_notification_origin(&url(
            "https://teams.cloud.microsoft/v2/"
        )));
        assert!(is_notification_origin(&url(
            "https://teams.microsoft.com/v2/"
        )));
        for target in [
            "https://login.microsoftonline.com/",
            "https://teams.cloud.microsoft.evil.com/",
            "http://teams.cloud.microsoft/",
        ] {
            assert!(!is_notification_origin(&url(target)));
        }
    }

    #[test]
    fn notification_payload_is_bounded_without_breaking_unicode() {
        let (_, title, body) = notification_payload(
            "😀".repeat(600),
            "é".repeat(5000),
            Some("page-test-1".into()),
        )
        .unwrap();
        assert_eq!(title.chars().count(), 512);
        assert_eq!(body.chars().count(), 4096);
        let (_, fallback, _) = notification_payload("  ".into(), String::new(), None).unwrap();
        assert_eq!(fallback, "Microsoft Teams");
        for id in ["", "bad/id", "not-an-id\n"] {
            assert!(notification_payload(String::new(), String::new(), Some(id.into())).is_err());
        }
    }

    #[test]
    fn microsoft_and_teams_hosts_are_trusted() {
        for host in [
            "https://teams.microsoft.com/v2/",
            "https://teams.cloud.microsoft/",
            "https://login.microsoftonline.com/common/oauth2/authorize",
            "https://login.live.com/",
            "https://substrate.office.com/api",
            "https://go.microsoft.com/fwlink/?linkid=1",
            "https://res-1.cdn.office.net/files/app.js",
            "https://aadcdn.msftauth.net/shared/1/oauth2",
        ] {
            assert!(is_trusted(&url(host)), "{host} should be trusted");
        }
    }

    #[test]
    fn organization_sso_stays_in_app_only_on_exact_https_host() {
        let settings = configuration::AppConfig {
            trusted_auth_hosts: vec![
                "login.example.com".into(),
                "identity.example.com".into(),
                "tenant-proxy.example.net".into(),
            ],
            ..Default::default()
        };
        let is_trusted = |url: &Url| settings.trusted(url);
        for target in [
            "https://login.example.com/",
            "https://login.example.com/login?returnUrl=https://teams.cloud.microsoft/",
            "https://identity.example.com/",
            "https://identity.example.com/adfs/ls/?client-request-id=example",
            "https://tenant-proxy.example.net/aad_login",
        ] {
            assert!(is_trusted(&url(target)), "{target} must stay in-app");
        }
        for target in [
            "http://login.example.com/",
            "http://identity.example.com/",
            "http://tenant-proxy.example.net/aad_login",
            "https://other-tenant.access.mcas.ms/",
            "https://sub.tenant-proxy.example.net/",
            "https://tenant-proxy.example.net.evil.com/",
            "https://tenant-proxy.example.net@evil.com/",
            "https://sub.identity.example.com/",
            "https://identity.example.com.evil.com/",
            "https://identity.example.com@evil.com/",
            "https://example.com/",
            "https://other.das.example.com/",
            "https://sub.login.example.com/",
            "https://login.example.com.evil.com/",
            "https://login.example.com@evil.com/",
        ] {
            assert!(!is_trusted(&url(target)), "{target} must not be trusted");
        }
    }

    #[test]
    fn teams_startup_and_cloud_redirects_stay_in_app() {
        assert!(is_trusted(&url(TEAMS_URL)));
        for target in [
            "https://teams.cloud.microsoft/",
            "https://teams.cloud.microsoft/v2/",
            "https://teams.cloud.microsoft/v2/?tenantId=example",
        ] {
            assert!(is_trusted(&url(target)), "{target} must stay in-app");
        }
        assert!(!is_trusted(&url("https://teams.cloud.microsoft.evil.com/")));
    }

    #[test]
    fn teams_cdn_is_trusted_so_its_report_pages_stay_in_app() {
        assert!(is_trusted(&url(
            "https://teams.public.onecdn.static.microsoft/teams-modular-packages/\
             react-web-client/1/report-third-party-cookies.html"
        )));
    }

    #[test]
    fn lookalike_and_third_party_hosts_are_not_trusted() {
        for host in [
            "https://evil.com/",
            "https://microsoft.com.evil.com/",
            "https://notmicrosoft.com/",
            "https://static.microsoft.evil.com/",
            "https://teams.microsoft.com.co/",
            "https://example.com/?redirect=https://teams.microsoft.com/",
        ] {
            assert!(!is_trusted(&url(host)), "{host} must not be trusted");
        }
    }

    #[test]
    fn non_web_schemes_are_not_trusted() {
        for target in [
            "mailto:someone@example.com",
            "tel:+123456",
            "javascript:alert(1)",
        ] {
            assert!(!is_trusted(&url(target)), "{target} must not be trusted");
        }
    }

    #[test]
    fn zero_count_leaves_icon_untouched() {
        let base = Image::new_owned([1, 2, 3, 255].repeat(16 * 16), 16, 16);
        let out = render_badge(&base, 0);
        assert_eq!(out.rgba(), base.rgba());
    }

    #[test]
    fn nonzero_count_draws_red_badge_with_white_text() {
        let out = render_badge(&blank(32), 3);
        let rgba = out.rgba();
        let red = rgba
            .chunks_exact(4)
            .any(|p| p[0] > 150 && p[1] < 100 && p[2] < 100);
        let white = rgba
            .chunks_exact(4)
            .any(|p| p[0] > 200 && p[1] > 200 && p[2] > 200);
        assert!(red, "expected a red badge");
        assert!(white, "expected white count text");
    }
}
