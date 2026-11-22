//! Camera / microphone / screen-recording permission state.
//!
//! macOS gates each behind a TCC prompt (`AVCaptureDevice`,
//! `CGPreflightScreenCaptureAccess`). Windows exposes the privacy toggles of
//! Settings > Privacy in the capability consent store; it has no prompt for
//! desktop apps and no screen-capture permission. Linux has neither (the
//! XDG portals ask per capture), so it reports `"unknown"`.

/// Device whose access is queried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediaKind {
    Camera,
    Microphone,
    Screen,
}

pub(super) fn parse_kind(op: &str, kind: &str) -> Result<MediaKind, String> {
    match kind {
        "camera" => Ok(MediaKind::Camera),
        "microphone" => Ok(MediaKind::Microphone),
        "screen" => Ok(MediaKind::Screen),
        other => Err(format!(
            "os.{op}: unknown media kind '{other}'; expected \"camera\", \"microphone\" or \"screen\""
        )),
    }
}

/// `"not-determined"`, `"granted"`, `"denied"`, `"restricted"` or
/// `"unknown"`.
pub(super) fn status(kind: MediaKind) -> &'static str {
    imp::status(kind)
}

/// Prompt for access when the user has not decided yet (macOS only);
/// resolves to whether access is granted.
pub(super) fn ask(kind: MediaKind) -> Result<bool, String> {
    imp::ask(kind)
}

/// Windows consent store values (`"Allow"` / `"Deny"`) of the device-wide
/// switch (HKLM), the user switch (HKCU) and the "let desktop apps access"
/// switch (HKCU `NonPackaged`), folded into one status.
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
fn windows_consent(
    machine: Option<&str>,
    user: Option<&str>,
    desktop_apps: Option<&str>,
) -> &'static str {
    let values = [machine, user, desktop_apps];
    if values
        .iter()
        .any(|value| value.is_some_and(|v| v.eq_ignore_ascii_case("deny")))
    {
        return "denied";
    }
    if user.is_some_and(|v| v.eq_ignore_ascii_case("allow")) {
        return "granted";
    }
    "unknown"
}

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
    };

    use super::MediaKind;

    const CONSENT_STORE: &str =
        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// `REG_SZ` value `Value` under `key\subkey`, `None` when absent.
    fn read(key: HKEY, subkey: &str) -> Option<String> {
        let subkey = wide(subkey);
        let name = wide("Value");
        let mut buffer = [0u16; 32];
        let mut bytes = u32::try_from(size_of_val(&buffer)).ok()?;
        // SAFETY: NUL-terminated subkey / value names; `buffer` holds
        // `bytes` writable bytes and RegGetValueW NUL-terminates REG_SZ.
        let status = unsafe {
            RegGetValueW(
                key,
                subkey.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &raw mut bytes,
            )
        };
        if status != 0 {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }

    pub(super) fn status(kind: MediaKind) -> &'static str {
        let capability = match kind {
            MediaKind::Camera => "webcam",
            MediaKind::Microphone => "microphone",
            // Desktop apps need no permission to capture the screen.
            MediaKind::Screen => return "granted",
        };
        let key = format!(r"{CONSENT_STORE}\{capability}");
        let machine = read(HKEY_LOCAL_MACHINE, &key);
        let user = read(HKEY_CURRENT_USER, &key);
        let desktop = read(HKEY_CURRENT_USER, &format!(r"{key}\NonPackaged"));
        super::windows_consent(machine.as_deref(), user.as_deref(), desktop.as_deref())
    }

    pub(super) fn ask(_kind: MediaKind) -> Result<bool, String> {
        Err(
            "os.askForMediaAccess: not supported on windows (Windows shows no permission \
             prompt to desktop apps; read os.getMediaAccessStatus and point the user to \
             Settings > Privacy)"
                .into(),
        )
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_av_foundation::{
        AVAuthorizationStatus, AVCaptureDevice, AVMediaType, AVMediaTypeAudio, AVMediaTypeVideo,
    };
    use objc2_core_graphics::{CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess};

    use super::MediaKind;

    fn media_type(kind: MediaKind) -> Option<&'static AVMediaType> {
        // SAFETY: AVFoundation framework constants, read-only.
        unsafe {
            match kind {
                MediaKind::Camera => AVMediaTypeVideo,
                MediaKind::Microphone => AVMediaTypeAudio,
                MediaKind::Screen => None,
            }
        }
    }

    pub(super) fn status(kind: MediaKind) -> &'static str {
        if kind == MediaKind::Screen {
            // Preflight cannot tell "never asked" from "denied".
            return if CGPreflightScreenCaptureAccess() {
                "granted"
            } else {
                "denied"
            };
        }
        let Some(media) = media_type(kind) else {
            return "unknown";
        };
        // SAFETY: `media` is AVMediaTypeVideo / AVMediaTypeAudio, the two
        // types this class method accepts.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) };
        match status {
            AVAuthorizationStatus::NotDetermined => "not-determined",
            AVAuthorizationStatus::Restricted => "restricted",
            AVAuthorizationStatus::Denied => "denied",
            AVAuthorizationStatus::Authorized => "granted",
            _ => "unknown",
        }
    }

    pub(super) fn ask(kind: MediaKind) -> Result<bool, String> {
        if kind == MediaKind::Screen {
            // Shows the system prompt the first time only; later calls just
            // report the current state.
            return Ok(CGRequestScreenCaptureAccess());
        }
        let media = media_type(kind).ok_or("os.askForMediaAccess: AVFoundation unavailable")?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let handler = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
            let _ = tx.try_send(granted.as_bool());
        });
        // SAFETY: valid media type; the completion block is retained by
        // AVFoundation and called once, on an arbitrary queue.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &handler) };
        rx.recv()
            .map_err(|_| "os.askForMediaAccess: the permission request was dropped".into())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use super::MediaKind;

    pub(super) fn status(_kind: MediaKind) -> &'static str {
        "unknown"
    }

    pub(super) fn ask(_kind: MediaKind) -> Result<bool, String> {
        Err(format!(
            "os.askForMediaAccess: not supported on {} (capture permissions are granted per \
             request by the desktop portal)",
            std::env::consts::OS
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_validated() {
        assert_eq!(parse_kind("x", "camera"), Ok(MediaKind::Camera));
        assert_eq!(parse_kind("x", "microphone"), Ok(MediaKind::Microphone));
        assert_eq!(parse_kind("x", "screen"), Ok(MediaKind::Screen));
        assert!(parse_kind("x", "Camera")
            .unwrap_err()
            .contains("unknown media kind"));
    }

    #[test]
    fn windows_consent_folding() {
        assert_eq!(windows_consent(None, Some("Allow"), None), "granted");
        assert_eq!(
            windows_consent(Some("Allow"), Some("Allow"), Some("Allow")),
            "granted"
        );
        assert_eq!(windows_consent(Some("Deny"), Some("Allow"), None), "denied");
        assert_eq!(windows_consent(None, Some("Allow"), Some("Deny")), "denied");
        assert_eq!(windows_consent(None, Some("deny"), None), "denied");
        assert_eq!(windows_consent(None, None, None), "unknown");
        assert_eq!(windows_consent(None, Some("Prompt"), None), "unknown");
    }
}
