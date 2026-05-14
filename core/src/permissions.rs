#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicStatus {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
    Unknown,
}

#[cfg(target_os = "macos")]
mod platform {
    use super::MicStatus;
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: *const AnyObject;
    }

    pub fn accessibility_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn mic_status() -> MicStatus {
        unsafe {
            let cls = class!(AVCaptureDevice);
            let raw: i64 = msg_send![cls, authorizationStatusForMediaType: AVMediaTypeAudio];
            match raw {
                0 => MicStatus::NotDetermined,
                1 => MicStatus::Restricted,
                2 => MicStatus::Denied,
                3 => MicStatus::Authorized,
                _ => MicStatus::Unknown,
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::MicStatus;
    pub fn accessibility_trusted() -> bool { true }
    pub fn mic_status() -> MicStatus { MicStatus::Authorized }
}

pub use platform::{accessibility_trusted, mic_status};

pub const PRIVACY_ACCESSIBILITY_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
