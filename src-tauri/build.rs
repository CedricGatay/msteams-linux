fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "set_unread_count",
            "show_notification",
            "notification_permission",
            "close_notification",
            "foreground_notifications_enabled",
            "configurator_status",
            "configurator_open_file",
            "configurator_export",
            "configurator_open_generated",
        ]),
    ))
    .expect("failed to run tauri-build");
}
