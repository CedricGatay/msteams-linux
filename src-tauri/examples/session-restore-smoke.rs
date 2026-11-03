//! Two-process macOS smoke test: synthetic cookie only, isolated profile/Keychain account.
//! Usage: MSTEAMS_RESTORE_SMOKE_DIR=/tmp/unique-dir cargo run --example session-restore-smoke -- seed
//! Then run the identical build with `restore`. Never uses real sign-in cookies.
#[cfg(target_os = "macos")]
#[path = "../src/configuration.rs"]
#[allow(dead_code)]
mod configuration;
#[cfg(target_os = "macos")]
#[path = "../src/session_restore.rs"]
#[allow(dead_code)] // Standalone smoke does not exercise every loading-page helper.
mod session_restore;

// Separate from the real Teams profile. Also used by the included module's tests.
#[cfg(target_os = "macos")]
const WEBKIT_PROFILE_ID: [u8; 16] = [
    0x75, 0x62, 0xf1, 0xba, 0xcc, 0x19, 0x49, 0xfe, 0x84, 0x0c, 0x12, 0x3e, 0xa2, 0xfd, 0xd8, 0x77,
];

#[cfg(target_os = "macos")]
fn main() {
    use session_restore::SessionRestore;
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};

    let seed = match std::env::args().nth(1).as_deref() {
        Some("seed") => true,
        Some("restore") => false,
        _ => panic!("expected seed or restore"),
    };
    let dir = PathBuf::from(
        std::env::var_os("MSTEAMS_RESTORE_SMOKE_DIR").expect("set a temporary smoke directory"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    let app = tauri::Builder::default().setup(move |app| {
        let state = SessionRestore::new(dir.clone(), WEBKIT_PROFILE_ID);
        state.set_enabled(true).unwrap();
        app.manage(state);
        session_restore::install_quit_menu(app.handle())?;
        let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(Url::parse("about:blank").unwrap()))
            .visible(false).data_store_identifier(WEBKIT_PROFILE_ID).build()?;
        let handle = app.handle().clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<(), ()> {
                if seed {
                    let cookie = tauri::webview::Cookie::build(("msteams_synthetic", "synthetic-not-an-auth-token"))
                        .domain("login.microsoftonline.com").path("/").secure(true).http_only(true).build();
                    window.set_cookie(cookie).map_err(|_| ())?;
                    if !window.cookies().map_err(|_| ())?.iter().any(|c| c.name() == "msteams_synthetic") {
                        return Err(());
                    }
                    eprintln!("[smoke] synthetic session cookie seeded; requesting clean quit");
                } else {
                    let before = window.cookies().map_err(|_| ())?;
                    if before.iter().any(|c| c.name() == "msteams_synthetic") {
                        eprintln!("[smoke] cookie unexpectedly survived without restoration");
                        return Err(());
                    }
                    handle.state::<SessionRestore>().navigate(&window, Url::parse("about:blank#restored").unwrap());
                    let deadline = Instant::now() + Duration::from_secs(8);
                    loop {
                        let cookies = window.cookies().map_err(|_| ())?;
                        if let Some(cookie) = cookies.into_iter().find(|c| c.name() == "msteams_synthetic") {
                            if cookie.value() != "synthetic-not-an-auth-token" || cookie.secure() != Some(true)
                                || cookie.http_only() != Some(true) || cookie.expires_datetime().is_some() {
                                return Err(());
                            }
                            window.delete_cookie(cookie).map_err(|_| ())?;
                            handle.state::<SessionRestore>().set_enabled(false)?;
                            eprintln!("[smoke] synthetic cookie restored with attributes intact and removed");
                            break;
                        }
                        if Instant::now() >= deadline { return Err(()); }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                Ok(())
            })();
            if result.is_err() { eprintln!("[smoke] FAILED"); }
            handle.exit(if result.is_ok() { 0 } else { 1 });
        });
        Ok(())
    }).build(tauri::generate_context!()).unwrap();
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            app.state::<SessionRestore>()
                .on_exit(app, &api, code.unwrap_or(0));
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Session restoration smoke test is macOS-only");
}
