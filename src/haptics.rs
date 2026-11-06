//! Taptic feedback. On iOS this drives UIKit's feedback generators through
//! the Objective-C runtime (no extra framework bindings needed). Elsewhere it
//! is a no-op, logged at debug level (JCODE_MOBILE_HAPTICS_LOG=1 prints it).
//!
//! Emit at action boundaries, after eligibility checks: Selection for a changed
//! choice, Light for navigation/send, Medium for a committed gesture, Rigid for
//! destructive actions, and notifications for meaningful outcomes. Ordinary
//! scrolling, typing, disabled controls and background polling stay silent.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Haptic {
    /// Small confirmations: send, threshold crossings.
    Light,
    /// Menus opening, committing a gesture.
    Medium,
    /// Destructive-ish taps such as closing a tab.
    Rigid,
    /// Picking a value from a list.
    Selection,
    Success,
    Warning,
    Error,
}

pub fn play(h: Haptic) {
    #[cfg(test)]
    CAPTURE.with(|events| {
        if let Some(events) = events.borrow_mut().as_mut() {
            events.push(h);
        }
    });
    if std::env::var_os("JCODE_MOBILE_HAPTICS_LOG").is_some() {
        eprintln!("haptic: {h:?}");
    }
    log::debug!("haptic {h:?}");
    imp::play(h);
}

#[cfg(test)]
thread_local! {
    static CAPTURE: std::cell::RefCell<Option<Vec<Haptic>>> = const { std::cell::RefCell::new(None) };
}

/// Record feedback on the calling UI thread without touching process-wide env.
#[cfg(test)]
pub(crate) fn capture<R>(f: impl FnOnce() -> R) -> (R, Vec<Haptic>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) { CAPTURE.with(|events| *events.borrow_mut() = None); }
    }
    CAPTURE.with(|events| {
        assert!(events.borrow().is_none(), "nested haptic capture");
        *events.borrow_mut() = Some(Vec::new());
    });
    let _reset = Reset;
    let result = f();
    let events = CAPTURE.with(|events| events.borrow_mut().take().unwrap());
    (result, events)
}

/// Success/error buzz when a turn finishes, given whether it was running.
pub fn turn_transition(was_processing: bool, s: &jcode_mobile_core::SessionState) {
    if !was_processing || s.is_processing {
        return;
    }
    let failed = s.error_banner.is_some()
        || matches!(s.phase, jcode_mobile_core::ConnectionPhase::Failed { .. });
    play(if failed { Haptic::Error } else { Haptic::Success });
}

#[cfg(target_os = "ios")]
mod imp {
    use super::Haptic;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::msg_send;

    pub fn play(h: Haptic) {
        // SAFETY: UIKit feedback generators are plain NSObjects used on the
        // main thread, which is where GPUI delivers input and frames.
        unsafe {
            match h {
                Haptic::Light | Haptic::Medium | Haptic::Rigid => {
                    let Some(cls) = AnyClass::get(c"UIImpactFeedbackGenerator") else { return };
                    // UIImpactFeedbackStyle: Light 0, Medium 1, Heavy 2, Soft 3, Rigid 4.
                    let style: isize = match h {
                        Haptic::Light => 0,
                        Haptic::Medium => 1,
                        _ => 4,
                    };
                    let a: Allocated<AnyObject> = msg_send![cls, alloc];
                    let g: Option<Retained<AnyObject>> = msg_send![a, initWithStyle: style];
                    if let Some(g) = g {
                        let _: () = msg_send![&*g, impactOccurred];
                    }
                }
                Haptic::Selection => {
                    let Some(cls) = AnyClass::get(c"UISelectionFeedbackGenerator") else { return };
                    let g: Option<Retained<AnyObject>> = msg_send![cls, new];
                    if let Some(g) = g {
                        let _: () = msg_send![&*g, selectionChanged];
                    }
                }
                Haptic::Success | Haptic::Warning | Haptic::Error => {
                    let Some(cls) = AnyClass::get(c"UINotificationFeedbackGenerator") else { return };
                    // UINotificationFeedbackType: Success 0, Warning 1, Error 2.
                    let kind: isize = match h {
                        Haptic::Success => 0,
                        Haptic::Warning => 1,
                        _ => 2,
                    };
                    let g: Option<Retained<AnyObject>> = msg_send![cls, new];
                    if let Some(g) = g {
                        let _: () = msg_send![&*g, notificationOccurred: kind];
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn play(_: super::Haptic) {}
}
