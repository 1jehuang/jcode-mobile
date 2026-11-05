//! System share sheet. iOS presents UIActivityViewController; the desktop
//! preview has none, so callers fall back to the clipboard.

/// Presents the share sheet for `text`. Returns false when there is no
/// system sheet (desktop preview) so the caller can copy instead.
pub fn share_text(text: &str) -> bool {
    imp::share_text(text)
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use objc2_foundation::NSString;

    pub fn share_text(text: &str) -> bool {
        // SAFETY: UIKit calls on the main thread with live, retained objects.
        unsafe {
            let (Some(app_cls), Some(arr_cls), Some(avc_cls)) = (
                AnyClass::get(c"UIApplication"),
                AnyClass::get(c"NSArray"),
                AnyClass::get(c"UIActivityViewController"),
            ) else {
                return false;
            };
            let app: *mut AnyObject = msg_send![app_cls, sharedApplication];
            if app.is_null() {
                return false;
            }
            let window: *mut AnyObject = msg_send![app, keyWindow];
            if window.is_null() {
                return false;
            }
            let mut top: *mut AnyObject = msg_send![window, rootViewController];
            if top.is_null() {
                return false;
            }
            loop {
                let next: *mut AnyObject = msg_send![top, presentedViewController];
                if next.is_null() {
                    break;
                }
                top = next;
            }
            let s = NSString::from_str(text);
            let items: *mut AnyObject = msg_send![arr_cls, arrayWithObject: &*s];
            let a: Allocated<AnyObject> = msg_send![avc_cls, alloc];
            let nil: *mut AnyObject = std::ptr::null_mut();
            let avc: Option<Retained<AnyObject>> =
                msg_send![a, initWithActivityItems: items, applicationActivities: nil];
            let Some(avc) = avc else { return false };
            let done: Option<&block2::Block<dyn Fn()>> = None;
            let _: () = msg_send![top, presentViewController: &*avc, animated: Bool::YES, completion: done];
            true
        }
    }
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn share_text(_: &str) -> bool {
        false
    }
}
