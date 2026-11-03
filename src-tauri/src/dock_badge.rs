//! Mirror the shell's unread counter onto this process's macOS Dock tile.
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSString};
use tauri::{AppHandle, Runtime};

fn label(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".into()),
    }
}

pub fn update<R: Runtime>(app: &AppHandle<R>, count: u32) {
    let label = label(count);
    let _ = app.run_on_main_thread(move || {
        if let Some(main_thread) = MainThreadMarker::new() {
            let dock = NSApplication::sharedApplication(main_thread).dockTile();
            let text = label.as_deref().map(NSString::from_str);
            dock.setBadgeLabel(text.as_deref());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clears_zero_and_matches_tray_overflow_format() {
        assert_eq!(label(0), None);
        assert_eq!(label(1).as_deref(), Some("1"));
        assert_eq!(label(99).as_deref(), Some("99"));
        assert_eq!(label(100).as_deref(), Some("99+"));
        assert_eq!(label(u32::MAX).as_deref(), Some("99+"));
    }
}
