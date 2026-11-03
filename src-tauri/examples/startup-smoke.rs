//! Native RELEASE smoke: actual Objective-C exception catching + data-document
//! creation/loading. Hidden, incognito, no network/credentials/user profile.
#[cfg(target_os = "macos")]
fn main() {
    use base64::Engine;
    use objc2::{
        exception,
        rc::{autoreleasepool, Retained},
    };
    use objc2_foundation::{NSException, NSString};
    use std::{panic::AssertUnwindSafe, time::Duration};
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

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
    println!("startup-smoke: Objective-C exception caught in release mode");

    let html = "<!doctype html><html><body data-smoke=\"ready\">Synthetic bootstrap</body></html>";
    let encoded = base64::engine::general_purpose::STANDARD.encode(html);
    let url: tauri::Url = format!("data:text/html;charset=utf-8;base64,{encoded}")
        .parse()
        .unwrap();
    tauri::Builder::default()
        .setup(move |app| {
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            WebviewWindowBuilder::new(app, "startup-smoke", WebviewUrl::External(url.clone()))
                .incognito(true)
                .visible(false)
                .on_page_load(|window, payload| {
                    if payload.event() != tauri::webview::PageLoadEvent::Finished {
                        return;
                    }
                    let handle = window.app_handle().clone();
                    if window
                        .eval_with_callback("document.body.dataset.smoke === 'ready'", move |raw| {
                            let ready = serde_json::from_str::<bool>(&raw).unwrap_or(false);
                            println!("startup-smoke: data document loaded={ready}");
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
