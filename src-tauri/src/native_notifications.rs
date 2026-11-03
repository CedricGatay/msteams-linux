//! macOS notifications under the real app identity, using UserNotifications.
//! Completion errors are generic: notification titles/bodies are never logged.
use block2::{DynBlock, RcBlock};
use objc2::{
    define_class, msg_send, rc::Retained, runtime::ProtocolObject, AllocAnyThread, DefinedClass,
};
use objc2_app_kit::NSAlert;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSString,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use std::{
    ptr::NonNull,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

const PREFIX: &str = "teams-shell:";
static AUTHORIZATION_LOCK: Mutex<()> = Mutex::new(());
struct DelegateIvars {
    on_click: Arc<dyn Fn(String) + Send + Sync>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MsteamsShellNotificationDelegate"]
    #[ivars = DelegateIvars]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}
    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            super::notification_trace::event("native_foreground_present");
            let mut options =
                UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List;
            if notification.request().content().sound().is_some() {
                options |= UNNotificationPresentationOptions::Sound;
            }
            completion.call((options,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_respond(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            // SAFETY: Apple's exported action identifier is a process-lifetime NSString.
            if &*response.actionIdentifier() == unsafe { UNNotificationDefaultActionIdentifier } {
                let identifier = response.notification().request().identifier().to_string();
                if let Some(id) = identifier.strip_prefix(PREFIX) {
                    super::notification_trace::event("native_clicked");
                    (self.ivars().on_click)(id.to_owned());
                }
            }
            completion.call(());
        }
    }
);

// SAFETY: the only ivar is an immutable Send + Sync callback. Delegate methods
// may run on Apple's background queue and use only thread-safe Rust/UI dispatch.
unsafe impl Send for NotificationDelegate {}
unsafe impl Sync for NotificationDelegate {}

pub struct NativeNotifications {
    _delegate: Retained<NotificationDelegate>,
}
impl NativeNotifications {
    pub fn new(on_click: impl Fn(String) + Send + Sync + 'static) -> Self {
        let allocated = NotificationDelegate::alloc().set_ivars(DelegateIvars {
            on_click: Arc::new(on_click),
        });
        // SAFETY: NSObject init initializes the freshly allocated subclass.
        let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(allocated), init] };
        if is_app_bundle() {
            UNUserNotificationCenter::currentNotificationCenter()
                .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        }
        Self {
            _delegate: delegate,
        }
    }
}

fn is_app_bundle() -> bool {
    NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".app")
}

fn permission_name(status: UNAuthorizationStatus) -> &'static str {
    match status {
        UNAuthorizationStatus::Authorized | UNAuthorizationStatus::Provisional => "granted",
        UNAuthorizationStatus::Denied => "denied",
        _ => "default",
    }
}

fn native_error(operation: &str, error: &NSError) -> String {
    // Do not include localizedDescription/userInfo: they may contain request data.
    format!(
        "macOS notification {operation} failed ({}, code {})",
        error.domain(),
        error.code()
    )
}

/// Must be dispatched to the main thread. Test failures must be visible, not
/// merely a false boolean in a terminal log the user cannot see.
pub fn show_test_error(message: &str) {
    if let Some(main_thread) = MainThreadMarker::new() {
        let alert = NSAlert::new(main_thread);
        alert.setMessageText(&NSString::from_str("Native notification test failed"));
        alert.setInformativeText(&NSString::from_str(message));
        alert.runModal();
    }
}

/// Called on a worker. Native callbacks never gate startup or block the UI.
pub fn permission(request: bool) -> Result<&'static str, String> {
    if !is_app_bundle() {
        return Err("Native notifications require the signed .app bundle. Launch with scripts/run-macos.sh or Finder, not cargo run.".into());
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    if request {
        let _authorization = AUTHORIZATION_LOCK
            .lock()
            .map_err(|_| "Notification authorization operation failed".to_owned())?;
        let current = permission(false)?;
        eprintln!("[msteams] native notification permission: {current}");
        if current != "default" {
            return Ok(current);
        }
        super::notification_trace::event("native_authorization_requested");
        let (tx, rx) = mpsc::channel();
        let callback = RcBlock::new(move |granted: objc2::runtime::Bool, error: *mut NSError| {
            let result = if error.is_null() {
                Ok(if granted.as_bool() {
                    "granted"
                } else {
                    "denied"
                })
            } else {
                // SAFETY: a non-null NSError is alive for this native callback.
                Err(native_error("authorization", unsafe { &*error }))
            };
            let _ = tx.send(result);
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Sound
                | UNAuthorizationOptions::Badge,
            &callback,
        );
        return rx
            .recv_timeout(Duration::from_secs(60))
            .map_err(|_| "macOS notification authorization timed out".to_owned())?;
    }
    let (tx, rx) = mpsc::channel();
    let callback = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        // SAFETY: Apple keeps the settings object alive for the callback duration.
        let status = unsafe { settings.as_ref() }.authorizationStatus();
        let _ = tx.send(permission_name(status));
    });
    center.getNotificationSettingsWithCompletionHandler(&callback);
    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "macOS notification settings timed out".to_owned())
}

pub fn show(id: &str, title: &str, body: &str, silent: bool) -> Result<(), String> {
    super::notification_trace::event("native_send");
    let permission = permission(false)
        .inspect_err(|_| super::notification_trace::event("native_permission_error"))?;
    if permission != "granted" {
        super::notification_trace::event("native_permission_blocked");
        return Err("Notifications are not allowed in macOS settings".to_owned());
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setThreadIdentifier(&NSString::from_str("teams-shell"));
    if !silent {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let identifier = NSString::from_str(&format!("{PREFIX}{id}"));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    let (tx, rx) = mpsc::channel();
    let callback = RcBlock::new(move |error: *mut NSError| {
        let result = if error.is_null() {
            super::notification_trace::event("native_accepted");
            Ok(())
        } else {
            super::notification_trace::event("native_rejected");
            // SAFETY: a non-null NSError is alive for this native callback.
            Err(native_error("delivery", unsafe { &*error }))
        };
        let _ = tx.send(result);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&callback));
    rx.recv_timeout(Duration::from_secs(10))
        .map_err(|_| "macOS notification delivery timed out".to_owned())?
}

pub fn close(id: &str) {
    super::notification_trace::event("native_close_requested");
    if !is_app_bundle() {
        return;
    }
    let identifiers = NSArray::from_slice(&[&*NSString::from_str(&format!("{PREFIX}{id}"))]);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removePendingNotificationRequestsWithIdentifiers(&identifiers);
    center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_error_reports_domain_and_code_without_error_descriptions() {
        // SAFETY: the domain is a valid NSString and no userInfo dictionary is supplied.
        let error = unsafe {
            NSError::errorWithDomain_code_userInfo(&NSString::from_str("UNErrorDomain"), 1, None)
        };
        assert_eq!(
            native_error("authorization", &error),
            "macOS notification authorization failed (UNErrorDomain, code 1)"
        );
    }

    #[test]
    fn permission_mapping_never_claims_undetermined_or_denied_is_granted() {
        assert_eq!(
            permission_name(UNAuthorizationStatus::NotDetermined),
            "default"
        );
        assert_eq!(permission_name(UNAuthorizationStatus::Denied), "denied");
        assert_eq!(
            permission_name(UNAuthorizationStatus::Authorized),
            "granted"
        );
        assert_eq!(
            permission_name(UNAuthorizationStatus::Provisional),
            "granted"
        );
    }
}
