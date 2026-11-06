//! Experimental screen/session availability policy. No input monitoring,
//! synthetic activity, idle timer reset, screenshots, or network requests.
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{
    NSDictionary, NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNumber,
    NSString,
};
use serde::Serialize;
use std::{
    path::PathBuf,
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    // Public Copy-rule API; CFDictionary is toll-free bridged to NSDictionary.
    fn CGSessionCopyCurrentDictionary() -> *mut NSDictionary<NSString, AnyObject>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub enabled: bool,
    pub known: bool,
    pub screensaver: bool,
    pub locked: bool,
    pub sleeping: bool,
    pub session_active: bool,
}
impl Snapshot {
    #[cfg(test)]
    fn inactive(self) -> bool {
        self.screensaver || self.locked || self.sleeping || !self.session_active
    }
}
#[derive(Clone, Copy)]
enum Signal {
    SaverOn,
    SaverOff,
    Lock,
    Unlock,
    Sleep,
    Wake,
    SessionOff,
    SessionOn,
}
fn transition(mut state: Snapshot, signal: Signal) -> Snapshot {
    state.known = true;
    match signal {
        Signal::SaverOn => state.screensaver = true,
        Signal::SaverOff => state.screensaver = false,
        Signal::Lock => state.locked = true,
        Signal::Unlock => state.locked = false,
        Signal::Sleep => state.sleeping = true,
        Signal::Wake => state.sleeping = false,
        Signal::SessionOff => state.session_active = false,
        Signal::SessionOn => state.session_active = true,
    }
    state
}
fn initial() -> Snapshot {
    // Copy ownership is compatible with NSObject release for the toll-free CF
    // dictionary. Inspect only two scalar state flags, never other session data.
    let dictionary = unsafe { Retained::from_raw(CGSessionCopyCurrentDictionary()) };
    let flag = |name| {
        dictionary
            .as_ref()?
            .objectForKey(&NSString::from_str(name))?
            .downcast_ref::<NSNumber>()
            .map(|number| number.boolValue())
    };
    // The public C constant kCGSessionOnConsoleKey expands to this spelling
    // (CGS, with an extra S), not to the constant's own identifier.
    let on_console = flag("kCGSSessionOnConsoleKey");
    let locked = flag("CGSSessionScreenIsLocked").unwrap_or(false);
    let saver = NSWorkspace::sharedWorkspace()
        .runningApplications()
        .iter()
        .any(|app| {
            app.bundleIdentifier()
                .is_some_and(|id| id.to_string() == "com.apple.ScreenSaver.Engine")
                && app.isActive()
        });
    Snapshot {
        known: on_console.is_some(),
        locked,
        screensaver: saver,
        session_active: on_console.unwrap_or(true),
        ..Snapshot::default()
    }
}
struct Inner {
    enabled: AtomicBool,
    observing: AtomicBool,
    state: Mutex<Snapshot>,
    preference: PathBuf,
    publish: Box<dyn Fn(Snapshot) + Send + Sync>,
}
fn enabled_preference(path: &std::path::Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<bool>(&bytes).ok())
        .unwrap_or(true)
}

#[derive(Clone)]
pub struct ScreenPresence(Arc<Inner>);
impl ScreenPresence {
    pub fn new(directory: PathBuf, publish: impl Fn(Snapshot) + Send + Sync + 'static) -> Self {
        let preference = directory.join("screen-presence-enabled.json");
        let enabled = enabled_preference(&preference);
        let result = Self(Arc::new(Inner {
            enabled: AtomicBool::new(enabled),
            observing: AtomicBool::new(false),
            state: Mutex::new(Snapshot::default()),
            preference,
            publish: Box::new(publish),
        }));
        if enabled && result.start_observing().is_err() {
            result.0.enabled.store(false, Ordering::SeqCst);
        }
        result
    }
    pub fn enabled(&self) -> bool {
        self.0.enabled.load(Ordering::SeqCst)
    }
    pub fn snapshot(&self) -> Snapshot {
        // Do not expose OS session state to the page when the policy is disabled.
        if !self.enabled() {
            return Snapshot::default();
        }
        let mut state = self.0.state.lock().map(|state| *state).unwrap_or_default();
        state.enabled = true;
        state
    }
    pub fn set_enabled(&self, enabled: bool) -> Result<(), ()> {
        if enabled {
            self.start_observing()?;
        }
        let temporary = self
            .0
            .preference
            .with_file_name(format!("screen-presence-{}.tmp", uuid::Uuid::new_v4()));
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let saved = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(if enabled { b"true" } else { b"false" })?;
            std::fs::rename(&temporary, &self.0.preference)
        })();
        if saved.is_err() {
            let _ = std::fs::remove_file(temporary);
            return Err(());
        }
        self.0.enabled.store(enabled, Ordering::SeqCst);
        (self.0.publish)(self.snapshot());
        Ok(())
    }
    fn signal(&self, signal: Signal) {
        let changed = if let Ok(mut state) = self.0.state.lock() {
            let next = transition(*state, signal);
            let changed = next != *state;
            *state = next;
            changed
        } else {
            false
        };
        // Never hold the state mutex while dispatching to WebKit's UI thread.
        if changed && self.enabled() {
            (self.0.publish)(self.snapshot());
        }
    }
    fn observe(&self, center: &NSNotificationCenter, name: &str, signal: Signal) {
        let state = self.clone();
        let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| state.signal(signal));
        // Foundation retains its block observer for the process lifetime. Install
        // only once; disabling gates publication/JS hooks, not an input monitor.
        // Captured Rust state is Send+Sync and notification payloads are ignored.
        let _token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(&NSString::from_str(name)),
                None,
                None,
                &block,
            )
        };
    }
    fn start_observing(&self) -> Result<(), ()> {
        if self.0.observing.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = objc2::exception::catch(std::panic::AssertUnwindSafe(|| {
            if let Ok(mut state) = self.0.state.lock() {
                *state = initial();
            }
            let distributed = NSDistributedNotificationCenter::defaultCenter();
            // Notification names are version-dependent macOS signals; this is
            // intentionally experimental and user-disableable, not a private API call.
            for (name, signal) in [
                ("com.apple.screensaver.didstart", Signal::SaverOn),
                ("com.apple.screensaver.didstop", Signal::SaverOff),
                ("com.apple.screenIsLocked", Signal::Lock),
                ("com.apple.screenIsUnlocked", Signal::Unlock),
            ] {
                self.observe(&distributed, name, signal);
            }
            let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
            for (name, signal) in [
                ("NSWorkspaceWillSleepNotification", Signal::Sleep),
                ("NSWorkspaceDidWakeNotification", Signal::Wake),
                (
                    "NSWorkspaceSessionDidResignActiveNotification",
                    Signal::SessionOff,
                ),
                (
                    "NSWorkspaceSessionDidBecomeActiveNotification",
                    Signal::SessionOn,
                ),
            ] {
                self.observe(&workspace, name, signal);
            }
        }));
        if result.is_err() {
            self.0.observing.store(false, Ordering::SeqCst);
            return Err(());
        }
        Ok(())
    }
}

/// Metadata-only support view; never serializes a provider object or presence
/// record, identifiers, names, preferences, tokens, or exception messages.
pub fn show_diagnostics(window: &tauri::WebviewWindow) {
    use tauri::Manager;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Stats {
        enabled: bool,
        native_known: bool,
        host_found: bool,
        hooked: bool,
        mapped_idle: bool,
        updates: u32,
        unsupported: bool,
    }
    let app = window.app_handle().clone();
    let _ = window.eval_with_callback("window.__msteamsScreenPresence ? window.__msteamsScreenPresence.getStats() : null", move |raw| {
        let text = match serde_json::from_str::<Stats>(&raw) {
            Ok(value) => format!("Enabled: {}\nNative state known: {}\nTeams provider found: {}\nBridge active: {}\nMapped inactive: {}\nNative updates applied: {}\nUnsupported/replaced hook: {}\n\nThis changes activity inputs, not your preferred availability. Server updates may be delayed.", value.enabled,value.native_known,value.host_found,value.hooked,value.mapped_idle,value.updates,value.unsupported),
            Err(_) => "Prototype status is unavailable on this page. Open Teams and enable the prototype in Help.".into(),
        };
        let _ = app.run_on_main_thread(move || {
            if let Some(main_thread) = objc2_foundation::MainThreadMarker::new() {
                let alert = objc2_app_kit::NSAlert::new(main_thread);
                alert.setMessageText(&NSString::from_str("Screen-saver presence prototype"));
                alert.setInformativeText(&NSString::from_str(&text));
                alert.runModal();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_inactivity_is_not_a_signal_only_real_screen_session_transitions_are() {
        let active = Snapshot {
            known: true,
            session_active: true,
            ..Snapshot::default()
        };
        assert!(!active.inactive());
        let saver = transition(active, Signal::SaverOn);
        assert!(saver.inactive());
        let locked = transition(saver, Signal::Lock);
        let saver_stopped = transition(locked, Signal::SaverOff);
        assert!(saver_stopped.inactive());
        let unlocked = transition(saver_stopped, Signal::Unlock);
        assert!(!unlocked.inactive());
        let sleep = transition(active, Signal::Sleep);
        assert!(sleep.inactive());
        assert!(!transition(sleep, Signal::Wake).inactive());
        assert!(transition(active, Signal::SessionOff).inactive());
    }
    #[test]
    fn defaults_on_but_persisted_boolean_opt_out_takes_priority() {
        let directory = std::env::temp_dir().join(format!(
            "screen-presence-preference-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("screen-presence-enabled.json");
        assert!(enabled_preference(&path));
        std::fs::write(&path, "false").unwrap();
        assert!(!enabled_preference(&path));
        std::fs::write(&path, "true").unwrap();
        assert!(enabled_preference(&path));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn explicit_opt_out_stays_disabled_and_snapshot_is_redacted() {
        let path =
            std::env::temp_dir().join(format!("screen-presence-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("screen-presence-enabled.json"), "false").unwrap();
        let state =
            ScreenPresence::new(path.clone(), |_| panic!("disabled state must not publish"));
        assert!(!state.enabled());
        assert_eq!(state.snapshot(), Snapshot::default());
        assert!(!state.0.observing.load(Ordering::SeqCst));
        std::fs::remove_dir_all(path).unwrap();
    }
}
