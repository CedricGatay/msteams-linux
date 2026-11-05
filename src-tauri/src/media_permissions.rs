//! Read-only macOS TCC authorization. Never creates a capture device/session,
//! requests access, enumerates devices, or opens the microphone/camera.
use objc2::{msg_send, runtime::AnyClass};
use objc2_foundation::NSString;
use serde::Serialize;

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    static AVMediaTypeAudio: &'static NSString;
    static AVMediaTypeVideo: &'static NSString;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Authorization {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
    Unknown,
}

fn authorization(value: isize) -> Authorization {
    match value {
        0 => Authorization::NotDetermined,
        1 => Authorization::Restricted,
        2 => Authorization::Denied,
        3 => Authorization::Authorized,
        _ => Authorization::Unknown,
    }
}

#[derive(Debug, Serialize)]
pub struct Status {
    pub audio: Authorization,
    pub video: Authorization,
}

pub fn status() -> Status {
    let Some(class) = AnyClass::get(c"AVCaptureDevice") else {
        return Status {
            audio: Authorization::Unknown,
            video: Authorization::Unknown,
        };
    };
    // Public, read-only AVFoundation class API (macOS 10.14+). AVMediaType
    // constants are immutable NSString objects exported by the linked framework;
    // AVAuthorizationStatus is NSInteger, represented by isize on this target.
    unsafe {
        let audio: isize = msg_send![class, authorizationStatusForMediaType: AVMediaTypeAudio];
        let video: isize = msg_send![class, authorizationStatusForMediaType: AVMediaTypeVideo];
        Status {
            audio: authorization(audio),
            video: authorization(video),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_only_actual_authorization_to_authorized() {
        assert_eq!(authorization(0), Authorization::NotDetermined);
        assert_eq!(authorization(1), Authorization::Restricted);
        assert_eq!(authorization(2), Authorization::Denied);
        assert_eq!(authorization(3), Authorization::Authorized);
        assert_eq!(authorization(-1), Authorization::Unknown);
        assert_eq!(authorization(4), Authorization::Unknown);
        let output = serde_json::to_value(Status {
            audio: Authorization::Authorized,
            video: Authorization::NotDetermined,
        })
        .unwrap();
        assert_eq!(
            output,
            serde_json::json!({"audio": "authorized", "video": "not-determined"})
        );
    }
}
