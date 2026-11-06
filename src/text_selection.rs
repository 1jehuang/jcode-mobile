//! Native read-only transcript selection. UIKit owns wrapping, handles,
//! the loupe, scrolling and the Copy menu without opening a keyboard.

#[cfg(target_os = "ios")]
pub fn present(text: &str) -> bool {
    use objc2::{
        msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, AnyObject, Bool},
    };
    use objc2_foundation::NSString;

    // SAFETY: called from a GPUI event on the main thread. The presented
    // controller retains its view and UIKit retains the presented controller.
    unsafe {
        let (
            Some(app_class),
            Some(controller_class),
            Some(text_class),
            Some(font_class),
            Some(color_class),
        ) = (
            AnyClass::get(c"UIApplication"),
            AnyClass::get(c"UIViewController"),
            AnyClass::get(c"UITextView"),
            AnyClass::get(c"UIFont"),
            AnyClass::get(c"UIColor"),
        )
        else {
            return false;
        };
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
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
        let allocated: Allocated<AnyObject> = msg_send![controller_class, alloc];
        let Some(controller): Option<Retained<AnyObject>> = msg_send![allocated, init] else {
            return false;
        };
        let allocated: Allocated<AnyObject> = msg_send![text_class, alloc];
        let Some(view): Option<Retained<AnyObject>> = msg_send![allocated, init] else {
            return false;
        };
        let text = NSString::from_str(text);
        let _: () = msg_send![&*view, setText: &*text];
        let _: () = msg_send![&*view, setEditable: Bool::NO];
        let _: () = msg_send![&*view, setSelectable: Bool::YES];
        let font: *mut AnyObject = msg_send![font_class, systemFontOfSize: 16.0f64];
        let color: *mut AnyObject = msg_send![color_class, systemBackgroundColor];
        let _: () = msg_send![&*view, setFont: font];
        let foreground: *mut AnyObject = msg_send![color_class, labelColor];
        let _: () = msg_send![&*view, setTextColor: foreground];
        let _: () = msg_send![&*view, setBackgroundColor: color];
        let _: () = msg_send![&*controller, setView: &*view];
        // Page sheet supports the standard swipe-down dismissal gesture.
        let _: () = msg_send![&*controller, setModalPresentationStyle: 1isize];
        let done: Option<&block2::Block<dyn Fn()>> = None;
        let _: () = msg_send![top, presentViewController: &*controller, animated: Bool::YES, completion: done];
        true
    }
}

#[cfg(not(target_os = "ios"))]
pub fn present(_: &str) -> bool {
    false
}
