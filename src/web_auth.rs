//! In-app web sign-in sheet (`ASWebAuthenticationSession`).
//!
//! Google refuses OAuth inside embedded web views, so the system
//! authentication sheet is the in-app option: it slides over the app, shares
//! Safari's cookies (the Google account chooser already lists the accounts
//! signed in on the phone) and is dismissed by [`close`] as soon as the device
//! flow is approved. Elsewhere [`open`] returns false and the caller falls
//! back to the system browser.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Present `url` in the system sign-in sheet. `dismissed` is set when the
/// sheet goes away (the person closed it, or [`close`] ran). Returns false
/// when no sheet could be shown.
pub fn open(url: &str, dismissed: Arc<AtomicBool>) -> bool {
    imp::open(url, dismissed)
}

/// Dismiss the sheet if one is showing. Main thread only.
pub fn close() {
    imp::close();
}

#[cfg(not(target_os = "ios"))]
mod imp {
    use super::*;

    pub fn open(_url: &str, _dismissed: Arc<AtomicBool>) -> bool {
        false
    }

    pub fn close() {}
}

#[cfg(target_os = "ios")]
mod imp {
    use super::*;
    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
    use objc2_foundation::NSString;
    use std::cell::RefCell;
    use std::sync::atomic::Ordering;

    #[link(name = "AuthenticationServices", kind = "framework")]
    unsafe extern "C" {}

    thread_local! {
        /// The session holds its context provider weakly, so both are kept here.
        static ACTIVE: RefCell<Option<(Retained<AnyObject>, Retained<Anchor>)>> = const { RefCell::new(None) };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "JcodeWebAuthAnchor"]
        struct Anchor;

        unsafe impl NSObjectProtocol for Anchor {}

        impl Anchor {
            #[unsafe(method(presentationAnchorForWebAuthenticationSession:))]
            fn presentation_anchor(&self, _session: *mut AnyObject) -> *mut AnyObject {
                key_window()
            }
        }
    );

    fn key_window() -> *mut AnyObject {
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(UIApplication), sharedApplication];
            if app.is_null() {
                return std::ptr::null_mut();
            }
            let windows: *mut AnyObject = msg_send![app, windows];
            if windows.is_null() {
                return std::ptr::null_mut();
            }
            let count: usize = msg_send![windows, count];
            let mut fallback: *mut AnyObject = std::ptr::null_mut();
            for i in 0..count {
                let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
                let key: bool = msg_send![window, isKeyWindow];
                if key {
                    return window;
                }
                if fallback.is_null() {
                    fallback = window;
                }
            }
            fallback
        }
    }

    pub fn open(url: &str, dismissed: Arc<AtomicBool>) -> bool {
        if MainThreadMarker::new().is_none() {
            return false;
        }
        close();
        unsafe {
            let nsurl: Option<Retained<AnyObject>> =
                msg_send![objc2::class!(NSURL), URLWithString: &*NSString::from_str(url)];
            let Some(nsurl) = nsurl else { return false };
            let flag = dismissed.clone();
            // Runs on success (never here: the flow ends on an https page) and
            // on every dismissal, including our own `cancel`.
            let done = RcBlock::new(move |_callback: *mut AnyObject, _error: *mut AnyObject| {
                flag.store(true, Ordering::Relaxed);
            });
            let alloc: Allocated<AnyObject> = msg_send![objc2::class!(ASWebAuthenticationSession), alloc];
            let session: Option<Retained<AnyObject>> = msg_send![
                alloc,
                initWithURL: &*nsurl,
                callbackURLScheme: &*NSString::from_str("jcode"),
                completionHandler: &*done,
            ];
            let Some(session) = session else { return false };
            let anchor: Retained<Anchor> = msg_send![Anchor::alloc(), init];
            let _: () = msg_send![&*session, setPresentationContextProvider: &*anchor];
            let started: bool = msg_send![&*session, start];
            if !started {
                log::warn!("web auth session did not start");
                return false;
            }
            ACTIVE.with(|active| *active.borrow_mut() = Some((session, anchor)));
            true
        }
    }

    pub fn close() {
        if MainThreadMarker::new().is_none() {
            return;
        }
        if let Some((session, _anchor)) = ACTIVE.with(|active| active.borrow_mut().take()) {
            let _: () = unsafe { msg_send![&*session, cancel] };
        }
    }
}
