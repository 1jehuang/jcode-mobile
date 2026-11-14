//! Photo attachments from the camera or the photo library.
//!
//! UIKit owns both pickers, so `JcodeBridge.framework` (ios-ext/, Media.swift)
//! presents them and hands back JPEGs that are already rotated upright and
//! scaled down. The bridge is loaded with `dlopen`, so an older bundle without
//! the media symbols degrades to "unavailable" instead of failing to launch.
//!
//! Android uses the system document picker and external camera, delivering
//! bounded, upright JPEGs through the JNI service bridge. Temporary camera
//! output is removed after delivery or cancellation.
//!
//! The desktop preview has no picker. `JCODE_MOBILE_ATTACH=<image file>` makes
//! either source deliver that file, for working on the composer from Linux.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use jcode_mobile_core::SessionImage;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Camera,
    Library,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Image(SessionImage),
    /// Every chosen photo was delivered.
    Done,
    Cancelled,
    /// No camera, permission denied, or nothing readable was chosen.
    Unavailable,
}

static TX: Mutex<Option<UnboundedSender<Event>>> = Mutex::new(None);

/// Channel of picker results. Call once from the app.
pub fn install() -> UnboundedReceiver<Event> {
    let (tx, rx) = unbounded();
    *TX.lock().unwrap() = Some(tx);
    rx
}

pub(crate) fn emit(event: Event) {
    if let Some(tx) = TX.lock().unwrap().as_ref() {
        let _ = tx.unbounded_send(event);
    }
}

/// Presents the picker for `source`, allowing up to `limit` photos. Returns
/// false when nothing could be presented. No event follows in that case.
pub fn pick(source: Source, limit: usize) -> bool {
    imp::pick(source, limit.max(1))
}

/// The media type gpui can decode for a transcript thumbnail.
pub fn format(media_type: &str) -> Option<gpui::ImageFormat> {
    Some(match media_type.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => gpui::ImageFormat::Jpeg,
        "image/png" => gpui::ImageFormat::Png,
        "image/webp" => gpui::ImageFormat::Webp,
        "image/gif" => gpui::ImageFormat::Gif,
        _ => return None,
    })
}

#[cfg(target_os = "ios")]
mod imp {
    use super::{Event, Source, emit};
    use jcode_mobile_core::SessionImage;
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::OnceLock;

    type Callback = extern "C" fn(i32, *const u8, usize, *const c_char);
    type Pick = unsafe extern "C" fn(i32, i32, Option<Callback>) -> i32;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    fn picker() -> Option<Pick> {
        static P: OnceLock<Option<Pick>> = OnceLock::new();
        // SAFETY: the symbol is a C function with exactly this signature
        // (ios-ext/Sources/JcodeBridge/Media.swift).
        *P.get_or_init(|| unsafe {
            let path = c"@executable_path/Frameworks/JcodeBridge.framework/JcodeBridge";
            let handle = dlopen(path.as_ptr(), 1);
            if handle.is_null() {
                return None;
            }
            let symbol = dlsym(handle, c"jcode_media_pick".as_ptr());
            (!symbol.is_null()).then(|| std::mem::transmute::<*mut c_void, Pick>(symbol))
        })
    }

    extern "C" fn on_media(status: i32, bytes: *const u8, len: usize, media_type: *const c_char) {
        emit(match status {
            0 => {
                if bytes.is_null() || len == 0 || len > jcode_mobile_core::session::MAX_IMAGE_BYTES {
                    return;
                }
                // SAFETY: Swift keeps `bytes[..len]` alive for this call. Copy now.
                let data = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
                let media_type = if media_type.is_null() {
                    "image/jpeg".into()
                } else {
                    unsafe { CStr::from_ptr(media_type) }.to_string_lossy().into_owned()
                };
                Event::Image(SessionImage::new(media_type, data))
            }
            1 => Event::Done,
            2 => Event::Cancelled,
            _ => Event::Unavailable,
        });
    }

    pub(super) fn pick(source: Source, limit: usize) -> bool {
        let Some(pick) = picker() else { return false };
        let source = if source == Source::Camera { 0 } else { 1 };
        // SAFETY: called on the main thread, as UIKit presentation requires.
        unsafe { pick(source, limit.min(i32::MAX as usize) as i32, Some(on_media)) == 1 }
    }
}

#[cfg(target_os = "android")]
mod imp {
    pub(super) fn pick(source: super::Source, limit: usize) -> bool {
        crate::android_services::pick(source == super::Source::Camera, limit)
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod imp {
    use super::{Event, Source, emit};
    use jcode_mobile_core::SessionImage;

    pub(super) fn pick(_: Source, _: usize) -> bool {
        let Some(path) = std::env::var_os("JCODE_MOBILE_ATTACH") else { return false };
        let path = std::path::PathBuf::from(path);
        let media_type = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/jpeg",
        };
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() && bytes.len() <= jcode_mobile_core::session::MAX_IMAGE_BYTES => {
                emit(Event::Image(SessionImage::new(media_type, bytes)));
                emit(Event::Done);
            }
            _ => emit(Event::Unavailable),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_decodable_media_types_get_a_thumbnail_format() {
        assert_eq!(format("image/jpeg"), Some(gpui::ImageFormat::Jpeg));
        assert_eq!(format(" IMAGE/PNG "), Some(gpui::ImageFormat::Png));
        assert_eq!(format("image/heic"), None);
        assert_eq!(format("application/pdf"), None);
    }
}
