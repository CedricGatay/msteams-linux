//! Opt-in, macOS-only session restoration. Secret payloads live only in Keychain.
//! No cookie is made persistent or given a new expiry. Server revocation still applies.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, ExitRequestApi, Manager, Runtime, Url, WebviewWindow,
};

/// The default macOS Quit item invokes NSApplication terminate:, bypassing
/// ExitRequested. Replace it with a custom Cmd-Q action routed through app.exit.
pub fn install_quit_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    menu.remove_at(0)?;
    let app_name = app.config().product_name.as_deref().unwrap_or("Teams");
    let quit = MenuItem::with_id(
        app,
        "quit-app",
        format!("Quit {app_name}"),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let app_menu = Submenu::with_items(
        app,
        app_name,
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    menu.insert(&app_menu, 0)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id.as_ref() == "quit-app" {
            app.exit(0);
        }
    });
    Ok(())
}

fn service_name() -> String {
    format!(
        "{}.session-restore",
        super::configuration::current().identifier()
    )
}
// Derive the Keychain account from the actual WebKit profile, including in tests.
fn profile_account(profile: [u8; 16]) -> String {
    let hex: String = profile.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "v1-{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
const MAX_AGE_SECONDS: u64 = 24 * 60 * 60;
const MAX_BYTES: usize = 256 * 1024;
const MAX_COOKIES: usize = 64;
const STARTUP_WAIT: Duration = Duration::from_secs(120);
pub const RESTORE_PAGE: &str = "restore.html";
const CONTINUE_URL: &str = "tauri://localhost/session-restore-continue";
const QUIT_WAIT: Duration = Duration::from_secs(5);
const ITEM_NOT_FOUND: i32 = -25300;

type Cookie = tauri::webview::Cookie<'static>;

// Deliberately no Debug implementation: this structure contains bearer secrets.
#[derive(Serialize, Deserialize)]
struct SavedSession {
    version: u8,
    saved_at: u64,
    cookies: Vec<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn eligible(cookie: &Cookie) -> bool {
    cookie.secure() == Some(true)
        && cookie.expires_datetime().is_none()
        && cookie.max_age().is_none()
        && !cookie.name().is_empty()
        && cookie.path().is_some_and(|path| path.starts_with('/'))
        && super::configuration::current().cookie_host_allowed(cookie.domain().unwrap_or(""))
}

fn encode(cookies: &[Cookie], saved_at: u64) -> Result<Option<Vec<u8>>, ()> {
    let cookies: Vec<_> = cookies
        .iter()
        .filter(|cookie| eligible(cookie))
        .map(ToString::to_string)
        .collect();
    if cookies.is_empty() {
        return Ok(None);
    }
    if cookies.len() > MAX_COOKIES {
        return Err(());
    }
    let bytes = serde_json::to_vec(&SavedSession {
        version: 1,
        saved_at,
        cookies,
    })
    .map_err(|_| ())?;
    if bytes.len() > MAX_BYTES {
        return Err(());
    }
    Ok(Some(bytes))
}

fn decode(bytes: &[u8], current_time: u64) -> Result<Vec<Cookie>, ()> {
    if bytes.len() > MAX_BYTES {
        return Err(());
    }
    let saved: SavedSession = serde_json::from_slice(bytes).map_err(|_| ())?;
    if saved.version != 1
        || saved.saved_at > current_time
        || current_time - saved.saved_at > MAX_AGE_SECONDS
        || saved.cookies.len() > MAX_COOKIES
    {
        return Err(());
    }
    saved
        .cookies
        .into_iter()
        .map(|raw| {
            let cookie = Cookie::parse(raw).map_err(|_| ())?.into_owned();
            if eligible(&cookie) {
                Ok(cookie)
            } else {
                Err(())
            }
        })
        .collect()
}

// Secrets are never included in status messages, preferences, or diagnostic files.
fn delete_saved(account: &str) -> Result<(), ()> {
    match delete_generic_password(&service_name(), account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
        Err(_) => Err(()),
    }
}

/// Do not consume a snapshot when a delayed Keychain response is no longer
/// needed. Native restoration never depends on a JavaScript callback.
fn load_active_snapshot(
    read: impl FnOnce() -> Result<Option<Vec<u8>>, ()>,
    delete: impl FnOnce() -> Result<(), ()>,
    active: impl Fn() -> bool,
    clock: impl Fn() -> u64,
) -> Result<Option<Vec<Cookie>>, ()> {
    if !active() {
        return Ok(None);
    }
    let Some(bytes) = read()? else {
        return Ok(None);
    };
    if !active() {
        return Ok(None);
    }
    let decoded = decode(&bytes, clock());
    if !active() {
        return Ok(None);
    }
    // Consume valid snapshots immediately before use; discard invalid/expired
    // snapshots too, but never delete an unused one after timeout/cancellation.
    delete()?;
    decoded.map(Some)
}

struct StartupNavigation {
    finished: Arc<AtomicBool>,
    restoring: AtomicBool,
    navigate: Box<dyn Fn() + Send + Sync>,
}

struct Inner {
    enabled: AtomicBool,
    generation: AtomicU64,
    // Serializes Keychain operations so disabling cannot race a later save.
    keychain: Mutex<()>,
    // 0 = running, 1 = saving before exit, 2 = exit is allowed.
    exit_phase: AtomicU8,
    preference: PathBuf,
    account: String,
    startup: Mutex<Option<Arc<StartupNavigation>>>,
}

#[derive(Clone)]
pub struct SessionRestore(Arc<Inner>);

impl SessionRestore {
    pub fn new(data_dir: PathBuf, profile: [u8; 16]) -> Self {
        let preference = data_dir.join("session-restore-enabled.json");
        let enabled = std::fs::read(&preference)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
            .unwrap_or(super::configuration::current().restore_session);
        Self(Arc::new(Inner {
            enabled: AtomicBool::new(enabled),
            generation: AtomicU64::new(0),
            keychain: Mutex::new(()),
            exit_phase: AtomicU8::new(0),
            preference,
            account: profile_account(profile),
            startup: Mutex::new(None),
        }))
    }

    pub fn enabled(&self) -> bool {
        self.0.enabled.load(Ordering::SeqCst)
    }

    /// Only a boolean preference is stored on disk. Disabling also deletes the
    /// Keychain payload, without logging out the currently running WebView.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), ()> {
        if !enabled {
            self.0.enabled.store(false, Ordering::SeqCst);
            self.0.generation.fetch_add(1, Ordering::SeqCst);
            self.continue_without_restoring();
            let state = self.clone();
            std::thread::spawn(move || {
                let Ok(_lock) = state.0.keychain.lock() else {
                    return;
                };
                if delete_saved(&state.0.account).is_err() {
                    eprintln!("[msteams] could not remove saved session from Keychain");
                }
            });
        }
        let temporary = self.0.preference.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            if enabled {
                b"true".as_slice()
            } else {
                b"false".as_slice()
            },
        )
        .and_then(|()| std::fs::rename(temporary, &self.0.preference))
        .map_err(|_| ())?;
        if enabled {
            self.0.generation.fetch_add(1, Ordering::SeqCst);
            self.0.enabled.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    /// Native work runs off the UI thread. The bundled loading page stays usable
    /// while a Keychain approval prompt is pending, with a manual skip and timeout.
    pub fn navigate<R: Runtime>(&self, window: &WebviewWindow<R>, url: Url) {
        if !self.enabled() {
            let _ = window.navigate(url);
            return;
        }
        let navigate_window = window.clone();
        let startup = Arc::new(StartupNavigation {
            finished: Arc::new(AtomicBool::new(false)),
            restoring: AtomicBool::new(false),
            navigate: Box::new(move || {
                let _ = navigate_window.navigate(url.clone());
            }),
        });
        if let Ok(mut pending) = self.0.startup.lock() {
            *pending = Some(startup.clone());
        }
        let timer_state = self.clone();
        let timer_startup = startup.clone();
        std::thread::spawn(move || {
            std::thread::sleep(STARTUP_WAIT);
            if timer_state.finish_startup(&timer_startup) {
                eprintln!("[msteams] session restore timed out; continuing without restoration");
            }
        });

        let state = self.clone();
        let generation = state.0.generation.load(Ordering::SeqCst);
        let window = window.clone();
        std::thread::spawn(move || {
            let active = || {
                state.enabled()
                    && !startup.finished.load(Ordering::SeqCst)
                    && state.0.generation.load(Ordering::SeqCst) == generation
            };
            let outcome = (|| {
                let _lock = state.0.keychain.lock().map_err(|_| ())?;
                if !active() {
                    return Ok(0);
                }
                // Read the native store before Keychain: it must not delay us
                // after the snapshot is consumed. No remote page is loaded yet.
                let existing = window.cookies().map_err(|_| ())?;
                let cookies = load_active_snapshot(
                    || match get_generic_password(&service_name(), &state.0.account) {
                        Ok(bytes) => Ok(Some(bytes)),
                        Err(error) if error.code() == ITEM_NOT_FOUND => {
                            eprintln!("[msteams] no saved session in Keychain");
                            Ok(None)
                        }
                        Err(_) => Err(()),
                    },
                    || {
                        // Once consumption begins, Quit must use the capture path
                        // rather than treating this as an untouched approval wait.
                        startup.restoring.store(true, Ordering::SeqCst);
                        delete_saved(&state.0.account)
                    },
                    active,
                    now,
                )?;
                let Some(cookies) = cookies else { return Ok(0) };
                eprintln!(
                    "[msteams] session snapshot loaded: {} cookies",
                    cookies.len()
                );
                let mut restored = 0;
                for cookie in cookies {
                    if !active() {
                        break;
                    }
                    if existing.iter().any(|current| same_cookie(current, &cookie)) {
                        continue;
                    }
                    window.set_cookie(cookie).map_err(|_| ())?;
                    restored += 1;
                }
                Ok::<_, ()>(restored)
            })();
            match outcome {
                Ok(count) => eprintln!("[msteams] session cookies restored: {count}"),
                Err(()) => eprintln!(
                    "[msteams] session restore unavailable; continuing with normal sign-in"
                ),
            }
            state.finish_startup(&startup);
        });
    }

    fn finish_startup(&self, startup: &Arc<StartupNavigation>) -> bool {
        let won = claim_navigation(&startup.finished);
        if won {
            (startup.navigate)();
        }
        if let Ok(mut pending) = self.0.startup.lock() {
            if pending
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, startup))
            {
                pending.take();
            }
        }
        won
    }

    pub fn continue_without_restoring(&self) -> bool {
        let pending = self
            .0
            .startup
            .lock()
            .ok()
            .and_then(|pending| pending.clone());
        pending.is_some_and(|startup| self.finish_startup(&startup))
    }

    fn cancel_pending_for_exit(&self) -> bool {
        let pending = self
            .0
            .startup
            .lock()
            .ok()
            .and_then(|pending| pending.clone());
        if let Some(startup) = pending {
            if !startup.restoring.load(Ordering::SeqCst) && claim_navigation(&startup.finished) {
                if let Ok(mut pending) = self.0.startup.lock() {
                    pending.take();
                }
                return true;
            }
        }
        false
    }

    pub fn is_continue_url(url: &Url) -> bool {
        url.as_str() == CONTINUE_URL
    }

    pub fn is_restore_page(url: &Url) -> bool {
        url.scheme() == "tauri"
            && url.host_str() == Some("localhost")
            && url.path() == "/restore.html"
            && url.query().is_none()
            && url.fragment().is_none()
    }

    /// Covers tray Quit and the custom application-menu/Cmd-Q action. A watchdog
    /// bounds shutdown even if
    /// a native cookie/Keychain operation never completes.
    pub fn on_exit<R: Runtime>(&self, app: &AppHandle<R>, api: &ExitRequestApi, code: i32) {
        if !self.enabled() || self.0.exit_phase.load(Ordering::SeqCst) == 2 {
            return;
        }
        if self.cancel_pending_for_exit() {
            // Quit while the Keychain prompt is still pending: don't replace the
            // saved authenticated session with the empty local loading page.
            self.0.exit_phase.store(2, Ordering::SeqCst);
            eprintln!("[msteams] quit during Keychain wait; preserving unused snapshot");
            return;
        }
        api.prevent_exit();
        eprintln!("[msteams] clean quit requested; capturing session");
        if self
            .0
            .exit_phase
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let state = self.clone();
        let timer_app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(QUIT_WAIT);
            state.finish_exit(&timer_app, code);
        });
        let state = self.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let saved = (|| {
                let _lock = state.0.keychain.lock().map_err(|_| ())?;
                // Clear any older snapshot first, including on a failed capture.
                delete_saved(&state.0.account)?;
                let window = app.get_webview_window("main").ok_or(())?;
                let cookies = window.cookies().map_err(|_| ())?;
                let eligible_count = cookies.iter().filter(|cookie| eligible(cookie)).count();
                eprintln!(
                    "[msteams] capture read {} cookies; {} eligible session cookies",
                    cookies.len(),
                    eligible_count
                );
                let payload = encode(&cookies, now())?;
                if state.enabled() && state.0.exit_phase.load(Ordering::SeqCst) == 1 {
                    if let Some(payload) = payload {
                        set_generic_password(&service_name(), &state.0.account, &payload)
                            .map_err(|_| ())?;
                        eprintln!("[msteams] session snapshot saved to Keychain: {eligible_count} cookies");
                    }
                }
                Ok::<_, ()>(())
            })();
            if saved.is_err() {
                eprintln!(
                    "[msteams] could not save session to Keychain; next launch may require sign-in"
                );
            }
            state.finish_exit(&app, code);
        });
    }

    fn finish_exit<R: Runtime>(&self, app: &AppHandle<R>, code: i32) {
        if self
            .0
            .exit_phase
            .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            app.exit(code);
        }
    }
}

fn same_cookie(a: &Cookie, b: &Cookie) -> bool {
    a.name() == b.name() && a.domain() == b.domain() && a.path() == b.path()
}

fn claim_navigation(finished: &AtomicBool) -> bool {
    !finished.swap(true, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::webview::cookie::{time::OffsetDateTime, SameSite};

    fn session(domain: &str) -> Cookie {
        // Synthetic aliases map to default Microsoft cookie hosts; prefix/suffix
        // mutations remain intact so exact-host rejection is still exercised.
        let domain = domain
            .replace("identity.example.com", "login.microsoftonline.com")
            .replace("login.example.com", "teams.microsoft.com");
        Cookie::build(("test-cookie", "synthetic-test-secret"))
            .domain(domain)
            .path("/")
            .secure(true)
            .http_only(true)
            .same_site(SameSite::Lax)
            .build()
    }

    fn pending_state(
        counter: Arc<std::sync::atomic::AtomicUsize>,
        restoring: bool,
    ) -> SessionRestore {
        SessionRestore(Arc::new(Inner {
            enabled: AtomicBool::new(true),
            generation: AtomicU64::new(0),
            keychain: Mutex::new(()),
            exit_phase: AtomicU8::new(0),
            preference: PathBuf::new(),
            account: "synthetic-test".into(),
            startup: Mutex::new(Some(Arc::new(StartupNavigation {
                finished: Arc::new(AtomicBool::new(false)),
                restoring: AtomicBool::new(restoring),
                navigate: Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            }))),
        }))
    }

    #[test]
    fn delayed_keychain_response_after_skip_does_not_consume_snapshot() {
        let cancelled = AtomicBool::new(false);
        let deleted = AtomicBool::new(false);
        let bytes = encode(&[session("identity.example.com")], 100)
            .unwrap()
            .unwrap();
        let result = load_active_snapshot(
            || {
                cancelled.store(true, Ordering::SeqCst);
                Ok(Some(bytes))
            },
            || {
                deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            || !cancelled.load(Ordering::SeqCst),
            || 101,
        )
        .unwrap();
        assert!(result.is_none());
        assert!(!deleted.load(Ordering::SeqCst));
    }

    #[test]
    fn active_restore_consumes_snapshot_before_returning_cookies() {
        let deleted = AtomicBool::new(false);
        let bytes = encode(&[session("identity.example.com")], 100)
            .unwrap()
            .unwrap();
        let result = load_active_snapshot(
            || Ok(Some(bytes)),
            || {
                deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            || true,
            || 101,
        )
        .unwrap()
        .unwrap();
        assert!(deleted.load(Ordering::SeqCst));
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn expiry_is_checked_after_the_keychain_prompt_not_before_it() {
        let clock = AtomicU64::new(100);
        let deleted = AtomicBool::new(false);
        let bytes = encode(&[session("identity.example.com")], 100)
            .unwrap()
            .unwrap();
        assert!(load_active_snapshot(
            || {
                clock.store(101 + MAX_AGE_SECONDS, Ordering::SeqCst);
                Ok(Some(bytes))
            },
            || {
                deleted.store(true, Ordering::SeqCst);
                Ok(())
            },
            || true,
            || clock.load(Ordering::SeqCst),
        )
        .is_err());
        assert!(deleted.load(Ordering::SeqCst));
    }

    #[test]
    fn manual_skip_navigates_once_without_waiting_for_keychain() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = pending_state(count.clone(), false);
        assert!(state.continue_without_restoring());
        assert!(!state.continue_without_restoring());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(state.0.startup.lock().unwrap().is_none());
    }

    #[test]
    fn quitting_during_approval_cancels_without_capturing_the_empty_loader() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = pending_state(count.clone(), false);
        assert!(state.cancel_pending_for_exit());
        assert!(!state.continue_without_restoring());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn quit_during_active_restore_keeps_the_normal_capture_path() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = pending_state(count, true);
        assert!(!state.cancel_pending_for_exit());
    }

    #[test]
    fn continue_route_is_exact_and_local_only() {
        let html = include_str!("../../dist/restore.html");
        assert!(html.contains(CONTINUE_URL));
        assert!(!html.contains("<script")); // Native work is never gated on a page callback.
        assert!(SessionRestore::is_continue_url(
            &Url::parse(CONTINUE_URL).unwrap()
        ));
        assert!(SessionRestore::is_restore_page(
            &Url::parse("tauri://localhost/restore.html").unwrap()
        ));
        for candidate in [
            "https://localhost/session-restore-continue",
            "tauri://other/session-restore-continue",
            "tauri://localhost/session-restore-continue?x=1",
        ] {
            assert!(!SessionRestore::is_continue_url(
                &Url::parse(candidate).unwrap()
            ));
        }
    }

    #[test]
    fn round_trip_preserves_session_and_security_attributes() {
        let cookie = session(".identity.example.com");
        let bytes = encode(&[cookie.clone()], 100).unwrap().unwrap();
        let restored = decode(&bytes, 101).unwrap();
        assert_eq!(restored[0].to_string(), cookie.to_string());
        assert!(restored[0].expires_datetime().is_none());
        assert!(restored[0].max_age().is_none());
    }

    #[test]
    fn captures_only_secure_session_cookies_on_exact_allowed_domains() {
        let mut insecure = session("identity.example.com");
        insecure.set_secure(false);
        let mut persistent = session("identity.example.com");
        persistent.set_expires(OffsetDateTime::now_utc());
        let mut max_age = session("identity.example.com");
        max_age.set_max_age(tauri::webview::cookie::time::Duration::hours(1));
        let cookies = [
            insecure,
            persistent,
            max_age,
            session("sub.identity.example.com"),
            session("identity.example.com.evil.com"),
            session("other.access.mcas.ms"),
        ];
        assert!(encode(&cookies, 100).unwrap().is_none());
    }

    #[test]
    fn rejects_stale_future_malformed_or_oversized_payloads() {
        let bytes = encode(&[session("identity.example.com")], 100)
            .unwrap()
            .unwrap();
        assert!(decode(&bytes, 100 + MAX_AGE_SECONDS + 1).is_err());
        assert!(decode(&bytes, 99).is_err());
        assert!(decode(b"not json", 100).is_err());
        assert!(decode(&vec![0; MAX_BYTES + 1], 100).is_err());
        assert!(encode(&vec![session("identity.example.com"); MAX_COOKIES + 1], 100).is_err());
    }

    #[test]
    fn rejects_unexpected_version_and_disallowed_cookies_on_load() {
        for (version, domain) in [(2, "identity.example.com"), (1, "evil.example")] {
            let bytes = serde_json::to_vec(&SavedSession {
                version,
                saved_at: 100,
                cookies: vec![session(domain).to_string()],
            })
            .unwrap();
            assert!(decode(&bytes, 100).is_err());
        }
    }

    #[test]
    fn empty_capture_produces_no_secret_payload() {
        assert!(encode(&[], 100).unwrap().is_none());
    }

    #[test]
    fn keychain_namespace_matches_the_fixed_webkit_profile() {
        let profile_hex: String = super::super::WEBKIT_PROFILE_ID
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            profile_account(super::super::WEBKIT_PROFILE_ID)
                .strip_prefix("v1-")
                .unwrap()
                .replace('-', ""),
            profile_hex
        );
    }

    #[test]
    fn oversized_capture_is_not_written_to_keychain() {
        let mut cookie = session("identity.example.com");
        cookie.set_value("x".repeat(MAX_BYTES));
        assert!(encode(&[cookie], 100).is_err());
    }

    #[test]
    fn saved_session_is_valid_only_within_the_bounded_restore_window() {
        let bytes = encode(&[session("identity.example.com")], 100)
            .unwrap()
            .unwrap();
        assert!(decode(&bytes, 100 + MAX_AGE_SECONDS).is_ok());
        assert!(decode(&bytes, 101 + MAX_AGE_SECONDS).is_err());
    }

    #[test]
    fn existing_cookie_identity_includes_name_domain_and_path() {
        let original = session("identity.example.com");
        assert!(same_cookie(&original, &original));
        let mut other_path = original.clone();
        other_path.set_path("/other");
        assert!(!same_cookie(&original, &other_path));
        assert!(!same_cookie(&original, &session("login.example.com")));
    }

    #[test]
    fn timeout_and_restore_completion_can_only_navigate_once() {
        let gate = Arc::new(AtomicBool::new(false));
        let other_gate = gate.clone();
        let worker = std::thread::spawn(move || claim_navigation(&other_gate));
        let timer_won = claim_navigation(&gate);
        let worker_won = worker.join().unwrap();
        assert_ne!(timer_won, worker_won);
        assert!(!claim_navigation(&gate));
    }

    #[test]
    #[ignore = "uses the local macOS Keychain with a synthetic test entry"]
    fn native_keychain_round_trip() {
        let service = "com.github.msteams.desktop.session-restore.tests";
        let account = format!("synthetic-{}-{}", std::process::id(), now());
        let synthetic = b"synthetic-session-test-not-a-real-token";
        set_generic_password(service, &account, synthetic).unwrap();
        let loaded = get_generic_password(service, &account);
        let deleted = delete_generic_password(service, &account);
        assert_eq!(loaded.unwrap(), synthetic);
        deleted.unwrap();
        assert_eq!(
            get_generic_password(service, &account).unwrap_err().code(),
            ITEM_NOT_FOUND
        );
    }

    #[test]
    fn opt_in_defaults_off_and_only_boolean_preference_is_persisted() {
        let dir = std::env::temp_dir().join(format!(
            "msteams-restore-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let state = SessionRestore::new(dir.clone(), super::super::WEBKIT_PROFILE_ID);
        assert!(!state.enabled());
        state.set_enabled(true).unwrap();
        assert!(state.enabled());
        assert_eq!(
            std::fs::read_to_string(&state.0.preference).unwrap(),
            "true"
        );
        assert!(SessionRestore::new(dir.clone(), super::super::WEBKIT_PROFILE_ID).enabled());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
