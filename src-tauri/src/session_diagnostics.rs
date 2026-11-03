//! Session diagnostics contain metadata only, never cookies or token values.
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Manager, Runtime, WebviewWindow};

#[derive(Default, Deserialize, Serialize)]
struct StorageCounts {
    local_entries: Option<u32>,
    session_entries: Option<u32>,
    local_msal_entries: Option<u32>,
    session_msal_entries: Option<u32>,
}

#[derive(Default, Serialize)]
struct CookieCounts {
    session: u32,
    persistent: u32,
    expired: u32,
}

fn cookie_counts(cookies: &[tauri::webview::Cookie<'_>]) -> BTreeMap<&'static str, CookieCounts> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let mut counts = BTreeMap::<_, CookieCounts>::new();
    for cookie in cookies {
        // Only fixed authentication/Teams domains; never serialize arbitrary
        // cookie domains, names, paths, identifiers, or values.
        let host = match cookie.domain().unwrap_or("").trim_start_matches('.') {
            "login.microsoftonline.com" => "login.microsoftonline.com",
            "teams.cloud.microsoft" => "teams.cloud.microsoft",
            "teams.microsoft.com" => "teams.microsoft.com",
            "login.example.com" => "login.example.com",
            "identity.example.com" => "identity.example.com",
            "tenant-proxy.example.net" => "tenant-proxy.example.net",
            _ => continue,
        };
        let entry = counts.entry(host).or_default();
        if let Some(expires) = cookie.expires_datetime() {
            entry.persistent += 1;
            if expires.unix_timestamp() <= now {
                entry.expired += 1;
            }
        } else {
            entry.session += 1;
        }
    }
    counts
}

/// Manual-only, best-effort diagnostics. Navigation and exit must never depend
/// on this callback arriving. Read cookies off the UI thread because WKWebView's
/// cookie API dispatches back to it.
pub fn snapshot<R: Runtime>(window: &WebviewWindow<R>, phase: &'static str) {
    let callback_window = window.clone();
    let result = window.eval_with_callback(include_str!("storage-counts.js"), move |raw| {
        // A typed decode drops unknown fields; raw JavaScript output is never logged.
        let storage = serde_json::from_str::<StorageCounts>(&raw).unwrap_or_default();
        let window = callback_window.clone();
        std::thread::spawn(move || {
            let cookies = window.cookies();
            let record = serde_json::json!({
                "phase": phase,
                "cookie_read_ok": cookies.is_ok(),
                "cookies": cookie_counts(cookies.as_deref().unwrap_or_default()),
                "storage": storage,
            });
            if let Ok(dir) = window.app_handle().path().app_data_dir() {
                let path = dir.join(format!("session-diagnostics-{phase}.json"));
                let temporary = path.with_extension("json.tmp");
                if let Ok(json) = serde_json::to_vec_pretty(&record) {
                    if std::fs::write(&temporary, json).is_ok()
                        && std::fs::rename(&temporary, &path).is_ok()
                    {
                        eprintln!("[msteams] session metadata saved: {}", path.display());
                    }
                }
            }
        });
    });
    if result.is_err() {
        eprintln!("[msteams] session metadata unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::webview::cookie::time::{Duration, OffsetDateTime};
    use tauri::webview::Cookie;

    #[test]
    fn summarizes_lifetimes_without_serializing_cookie_secrets() {
        let cookies = [
            Cookie::build(("secret-name", "secret-value"))
                .domain(".login.microsoftonline.com")
                .build(),
            Cookie::build(("another-secret", "another-value"))
                .domain("login.microsoftonline.com")
                .expires(OffsetDateTime::now_utc() + Duration::days(1))
                .build(),
            Cookie::build(("expired", "private"))
                .domain("login.microsoftonline.com")
                .expires(OffsetDateTime::now_utc() - Duration::days(1))
                .build(),
            Cookie::build(("unrelated", "private"))
                .domain("unrelated.example")
                .build(),
        ];
        let counts = cookie_counts(&cookies);
        assert_eq!(counts.len(), 1);
        let login = &counts["login.microsoftonline.com"];
        assert_eq!(login.session, 1);
        assert_eq!(login.persistent, 2);
        assert_eq!(login.expired, 1);
        let output = serde_json::to_string(&counts).unwrap();
        for forbidden in ["secret", "value", "private", "unrelated"] {
            assert!(!output.contains(forbidden));
        }
    }
}
