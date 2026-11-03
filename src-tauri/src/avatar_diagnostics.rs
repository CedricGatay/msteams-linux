//! Manual avatar diagnostics: fixed numeric/boolean schema, never image/user data.
use serde::{Deserialize, Serialize};
use tauri::{Manager, Runtime, WebviewWindow};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Counts {
    image_scanned: u32,
    image_loaded: u32,
    image_failed: u32,
    image_pending: u32,
    avatar_containers: u32,
    avatar_images: u32,
    avatar_loaded: u32,
    avatar_failed: u32,
    avatar_pending: u32,
    failed_avatar_blob: u32,
    failed_avatar_https: u32,
    failed_avatar_data: u32,
    failed_avatar_other: u32,
    loaded_avatar_blob: u32,
    loaded_avatar_https: u32,
    loaded_avatar_svg_data: u32,
    loaded_avatar_other_data: u32,
    loaded_avatar_other: u32,
    avatar_tiny_image: u32,
    avatar_css_hidden: u32,
    avatar_css_transparent: u32,
    avatar_no_box: u32,
    avatar_in_view: u32,
    avatar_offscreen: u32,
    photo_context_found: bool,
    photo_base_url_present: bool,
    photo_cookie_state_known: bool,
    photo_cookie_valid: bool,
    media_auth_service_found: bool,
    media_auth_initialized: bool,
    media_auth_setup_in_progress: bool,
    media_auth_base_url_present: bool,
    media_auth_cookie_state_known: bool,
    media_auth_cookie_valid: bool,
    photo_requests: u32,
    photo_http_success: u32,
    photo_http_401: u32,
    photo_http_403: u32,
    photo_http_404: u32,
    photo_http_429: u32,
    photo_http_server_error: u32,
    photo_http_other: u32,
    photo_http_unknown: u32,
    resource_timing_available: bool,
    service_worker_controlled: bool,
    image_bitmap_available: bool,
    image_decoder_available: bool,
}

#[cfg(feature = "avatar-probe")]
fn private_probe(path: &std::path::Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 65536 || metadata.permissions().mode() & 0o077 != 0 {
        return None;
    }
    let source = std::fs::read_to_string(path).ok()?;
    (source.len() <= 65536).then_some(source)
}

fn script<R: Runtime>(window: &WebviewWindow<R>) -> String {
    // Deliberately unavailable in normal builds. Private probes live outside
    // both the checkout and signed bundle, and are explicitly local executable
    // code for developer diagnosis, not a configurable production feature.
    #[cfg(feature = "avatar-probe")]
    {
        if let Ok(directory) = window.app_handle().path().app_data_dir() {
            if let Some(source) = private_probe(&directory.join("avatar-diagnostics-probe.js")) {
                return source;
            }
        }
    }
    let _ = window;
    include_str!("avatar-diagnostics.js").into()
}

/// Best effort; cannot gate navigation/exit or modify page/network behavior.
pub fn snapshot<R: Runtime>(window: &WebviewWindow<R>) {
    if window.label() != "main"
        || !window
            .url()
            .is_ok_and(|url| super::is_notification_origin(&url))
    {
        return;
    }
    let handle = window.app_handle().clone();
    let result = window.eval_with_callback(script(window), move |raw| {
        let Ok(counts) = serde_json::from_str::<Counts>(&raw) else {
            eprintln!("[msteams] avatar metadata unavailable");
            return;
        };
        // Never log raw JS output, parser errors, DOM data or output paths.
        let handle = handle.clone();
        std::thread::spawn(move || {
            let saved = (|| -> Result<(), Box<dyn std::error::Error>> {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let directory = handle.path().app_data_dir()?;
                let temporary =
                    directory.join(format!("avatar-diagnostics-{}.tmp", uuid::Uuid::new_v4()));
                let bytes = serde_json::to_vec_pretty(&counts)?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&temporary)?;
                let written = file.write_all(&bytes).and_then(|()| {
                    std::fs::rename(&temporary, directory.join("avatar-diagnostics.json"))
                });
                if written.is_err() {
                    let _ = std::fs::remove_file(temporary);
                }
                written?;
                Ok(())
            })()
            .is_ok();
            eprintln!("[msteams] avatar metadata saved={saved}");
        });
    });
    if result.is_err() {
        eprintln!("[msteams] avatar metadata unavailable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "avatar-probe")]
    #[test]
    fn private_probe_rejects_shared_files_symlinks_and_oversized_sources() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!("avatar-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("probe.js");
        std::fs::write(&path, "({})").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(private_probe(&path).as_deref(), Some("({})"));
        let link = root.join("link.js");
        symlink(&path, &link).unwrap();
        assert!(private_probe(&link).is_none());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(private_probe(&path).is_none());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&path, " ".repeat(65537)).unwrap();
        assert!(private_probe(&path).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn diagnostics_only_accept_known_counts_and_booleans() {
        let mut data = serde_json::to_value(Counts::default()).unwrap();
        assert!(serde_json::from_value::<Counts>(data.clone()).is_ok());
        data["avatar_failed"] = serde_json::json!("private value");
        assert!(serde_json::from_value::<Counts>(data.clone()).is_err());
        data["avatar_failed"] = serde_json::json!(2);
        data["url"] = serde_json::json!("https://example.com/private-photo?secret=token");
        assert!(serde_json::from_value::<Counts>(data).is_err());
        assert!(serde_json::from_value::<Counts>(serde_json::Value::Null).is_err());
    }
}
