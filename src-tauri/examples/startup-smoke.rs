//! Native RELEASE smoke: actual Objective-C exception catching + data-document
//! creation/loading/reload and native menu wiring. Hidden, incognito, no
//! network/credentials/user profile.
#[cfg(target_os = "macos")]
#[path = "../src/media_permissions.rs"]
#[allow(dead_code)]
mod media_permissions;
#[cfg(target_os = "macos")]
#[path = "../src/menu_bar.rs"]
#[allow(dead_code)]
mod menu_bar;

#[cfg(target_os = "macos")]
fn main() {
    use base64::Engine;
    use objc2::{
        exception,
        rc::{autoreleasepool, Retained},
    };
    use objc2_foundation::{NSException, NSString};
    use std::{
        panic::AssertUnwindSafe,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tauri::{
        menu::{Menu, MenuItem, MenuItemKind, HELP_SUBMENU_ID},
        Manager, WebviewUrl, WebviewWindowBuilder,
    };

    #[cfg(panic = "abort")]
    compile_error!("This release smoke requires panic=unwind, as does Wry's exception catcher");

    autoreleasepool(|_| {
        let name = NSString::from_str("TeamsSyntheticStartupException");
        let reason =
            NSString::from_str("Synthetic URL-task cancellation for release regression test");
        // No userInfo, hence no Foundation generic argument invariants to uphold.
        let native =
            unsafe { NSException::exceptionWithName_reason_userInfo(&name, Some(&reason), None) };
        // Exception is a transparent wrapper around any Objective-C object.
        let native: Retained<exception::Exception> = unsafe { Retained::cast_unchecked(native) };
        assert!(exception::catch(AssertUnwindSafe(|| exception::throw(native))).is_err());
    });
    println!(
        "startup-smoke: Objective-C exception caught ({} mode)",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let status = autoreleasepool(|_| media_permissions::status());
    assert!(serde_json::to_value(status).unwrap().is_object());
    println!("startup-smoke: read-only media authorization query completed (no capture)");

    let html = "<!doctype html><html><body data-smoke=\"ready\">Synthetic bootstrap</body></html>";
    let encoded = base64::engine::general_purpose::STANDARD.encode(html);
    let url: tauri::Url = format!("data:text/html;charset=utf-8;base64,{encoded}")
        .parse()
        .unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    tauri::Builder::default()
        .setup(move |app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            app.set_menu(Menu::default(app.handle())?)?;
            let actions = Menu::with_items(
                app,
                &[&MenuItem::with_id(
                    app,
                    "smoke-help-action",
                    "Synthetic Help Action",
                    true,
                    None::<&str>,
                )?],
            )?;
            let directory =
                std::env::temp_dir().join(format!("teams-menu-smoke-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory)?;
            let checkbox = menu_bar::install(app.handle(), &actions, directory)?;
            assert!(checkbox.is_checked()?);
            assert!(actions.items()?.is_empty());
            let root = app.menu().unwrap();
            let Some(MenuItemKind::Submenu(help)) = root.get(HELP_SUBMENU_ID) else {
                panic!("Help menu missing");
            };
            assert!(help.get("smoke-help-action").is_some());
            let view = root
                .items()?
                .into_iter()
                .find_map(|item| match item {
                    MenuItemKind::Submenu(menu) if menu.text().ok().as_deref() == Some("View") => {
                        Some(menu)
                    }
                    _ => None,
                })
                .expect("View menu missing");
            assert!(view.get(menu_bar::RELOAD_ID).is_some());
            assert!(view.get(menu_bar::TRAY_ID).is_some());
            println!("startup-smoke: native Help/View menu wiring verified");
            WebviewWindowBuilder::new(app, "startup-smoke", WebviewUrl::External(url.clone()))
                .incognito(true)
                .disable_drag_drop_handler()
                .visible(false)
                .on_page_load(move |window, payload| {
                    if payload.event() != tauri::webview::PageLoadEvent::Finished {
                        return;
                    }
                    if loads.fetch_add(1, Ordering::SeqCst) == 0 {
                        if window.reload().is_err() {
                            window.app_handle().exit(1);
                        }
                        return;
                    }
                    let handle = window.app_handle().clone();
                    if window
                        .eval_with_callback("document.body.dataset.smoke === 'ready'", move |raw| {
                            let ready = serde_json::from_str::<bool>(&raw).unwrap_or(false);
                            println!("startup-smoke: data document loaded and reloaded={ready}");
                            handle.exit(if ready { 0 } else { 1 });
                        })
                        .is_err()
                    {
                        window.app_handle().exit(1);
                    }
                })
                .build()?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(15));
                handle.exit(2);
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("native startup smoke failed to build")
        .run(|_, _| {});
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("startup-smoke is macOS-only");
}
