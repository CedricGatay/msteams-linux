//! Opt-in notification diagnostics. Only fixed labels, booleans and counters
//! are accepted from JavaScript; never log raw eval output or message contents.
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager, Runtime, Url};

pub fn enabled() -> bool {
    std::env::var("MSTEAMS_NOTIFICATION_TRACE").as_deref() == Ok("1")
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn write_record(record: serde_json::Value) {
    use std::io::Write;
    // Never panic across an Objective-C delegate callback if stderr is closed.
    let _ = writeln!(
        std::io::stderr().lock(),
        "[msteams-notification-trace] {record}"
    );
}

pub fn event(stage: &'static str) {
    if enabled() {
        write_record(serde_json::json!({"t_ms": timestamp(), "stage": stage}));
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Permission {
    Default,
    Granted,
    Denied,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Stage {
    BridgeReady,
    Permission,
    PermissionRead,
    PermissionRequest,
    PermissionError,
    WindowNotification,
    ServiceWorkerPage,
    TeamsForeground,
    SendAttempt,
    DeliveryAccepted,
    DeliveryFailed,
    Closed,
    NativeClick,
    NativeRetained,
    DomAlertActivity,
    ButtonClick,
    Focus,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct UiEvent {
    stage: Stage,
    t_ms: u64,
    focused: bool,
    visible: bool,
    permission: Permission,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct TeamsAdapterSnapshot {
    enabled: bool,
    react_root_found: bool,
    handler_found: bool,
    hooked: bool,
    in_app_service_found: bool,
    discovery_attempts: u64,
    discovery_exhausted: bool,
    browser_attempts: u64,
    queued: u64,
    skipped_focus: u64,
    skipped_permissions: u64,
    other_results: u64,
    original_errors: u64,
    mirrored: u64,
    mirror_errors: u64,
    unsupported: u64,
    activation_errors: u64,
    in_app_calls: u64,
    in_app_tests: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    notification_hooked: bool,
    service_worker_page_hooked: bool,
    worker_controlled: bool,
    iframe_count: u64,
    permission: Permission,
    focused: bool,
    visible: bool,
    window_calls: u64,
    service_worker_page_calls: u64,
    #[serde(default)]
    teams_foreground_calls: u64,
    #[serde(default)]
    teams_adapter: Option<TeamsAdapterSnapshot>,
    send_attempts: u64,
    accepted: u64,
    failed: u64,
    closed: u64,
    #[serde(default)]
    retained_after_close: u64,
    clicks: u64,
    permission_errors: u64,
    permission_reads: u64,
    permission_request_calls: u64,
    dom_alert_batches: u64,
    button_clicks: u64,
    title_unread: u64,
    events: Vec<UiEvent>,
}

fn safe_snapshot(raw: &str) -> Option<Snapshot> {
    if raw.len() > 64 * 1024 {
        return None;
    }
    let mut snapshot = serde_json::from_str::<Option<Snapshot>>(raw)
        .ok()
        .flatten()?;
    if snapshot.events.len() > 64 {
        snapshot.events.drain(..snapshot.events.len() - 64);
    }
    Some(snapshot)
}

#[derive(Clone, Copy, PartialEq, Serialize, Debug)]
#[serde(rename_all = "snake_case")]
enum Page {
    Teams,
    Restoring,
    SignIn,
    Other,
}

fn page(url: &Url) -> Page {
    if super::SessionRestore::is_restore_page(url) {
        return Page::Restoring;
    }
    match url.host_str() {
        Some("teams.cloud.microsoft" | "teams.microsoft.com") if url.scheme() == "https" => {
            Page::Teams
        }
        Some(
            "login.microsoftonline.com"
            | "login.live.com"
            | "identity.example.com"
            | "login.example.com"
            | "tenant-proxy.example.net",
        ) => Page::SignIn,
        _ => Page::Other,
    }
}

/// Passive polling also works when notification IPC is denied or Teams replaces
/// the JS hook. A missing eval callback never blocks navigation, exit, or the UI.
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    if !enabled() {
        return;
    }
    event("trace_started");
    let app = app.clone();
    std::thread::spawn(move || {
        let cache = Arc::new(Mutex::new(None::<(Option<Snapshot>, Option<bool>)>));
        let mut previous_page = None;
        loop {
            let Some(window) = app.get_webview_window("main") else {
                break;
            };
            let current = window.url().map(|url| page(&url)).unwrap_or(Page::Other);
            if previous_page != Some(current) {
                write_record(
                    serde_json::json!({"t_ms": timestamp(), "stage": "page", "page": current}),
                );
                previous_page = Some(current);
                if let Ok(mut cache) = cache.lock() {
                    *cache = None;
                }
            }
            if current == Page::Teams {
                let cache = cache.clone();
                let window_focused = window.is_focused().ok();
                let result = window.eval_with_callback(
                    "window.__msteamsNotificationTraceSnapshot ? window.__msteamsNotificationTraceSnapshot() : null",
                    move |raw| {
                        let snapshot = (safe_snapshot(&raw), window_focused);
                        if let Ok(mut previous) = cache.lock() {
                            if previous.as_ref() != Some(&snapshot) {
                                write_record(serde_json::json!({
                                    "t_ms": timestamp(), "stage": "snapshot", "window_focused": snapshot.1, "state": snapshot.0,
                                }));
                                *previous = Some(snapshot);
                            }
                        }
                    },
                );
                if result.is_err() {
                    event("snapshot_request_failed");
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata() -> serde_json::Value {
        serde_json::json!({
            "notification_hooked": true, "service_worker_page_hooked": true,
            "worker_controlled": false, "iframe_count": 0, "permission": "granted", "focused": true, "visible": true,
            "window_calls": 1, "service_worker_page_calls": 0, "send_attempts": 1, "accepted": 1,
            "failed": 0, "closed": 0, "clicks": 0, "permission_errors": 0, "permission_reads": 0, "permission_request_calls": 0,
            "dom_alert_batches": 1, "button_clicks": 1, "title_unread": 0,
            "events": [{"stage": "window_notification", "t_ms": 1, "focused": true, "visible": true, "permission": "granted"}]
        })
    }
    #[test]
    fn accepts_only_typed_metadata_and_rejects_content_or_unknown_labels() {
        assert!(safe_snapshot(&metadata().to_string()).is_some());
        let mut content = metadata();
        content["body"] = serde_json::json!("synthetic-private-content");
        assert!(safe_snapshot(&content.to_string()).is_none());
        let mut unknown = metadata();
        unknown["events"][0]["stage"] = serde_json::json!("synthetic-private-event-name");
        assert!(safe_snapshot(&unknown.to_string()).is_none());
        assert!(safe_snapshot("not json").is_none());
        assert!(safe_snapshot("null").is_none());
        assert!(safe_snapshot(&"x".repeat(64 * 1024 + 1)).is_none());
    }
    #[test]
    fn bounds_event_history() {
        let mut input = metadata();
        input["events"] = serde_json::json!(vec![input["events"][0].clone(); 80]);
        assert_eq!(safe_snapshot(&input.to_string()).unwrap().events.len(), 64);
    }
    #[test]
    fn classifies_pages_without_returning_paths_queries_or_tenant_names() {
        assert_eq!(
            page(&Url::parse("https://teams.cloud.microsoft/v2/?sensitive=synthetic").unwrap()),
            Page::Teams
        );
        assert_eq!(
            page(&Url::parse("tauri://localhost/restore.html").unwrap()),
            Page::Restoring
        );
        assert_eq!(
            page(&Url::parse("https://identity.example.com/adfs/ls/?token=synthetic").unwrap()),
            Page::SignIn
        );
        assert_eq!(
            page(&Url::parse("https://other.example/private").unwrap()),
            Page::Other
        );
    }
}
