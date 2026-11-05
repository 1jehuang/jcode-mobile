//! System notifications. On iOS these are local `UNUserNotificationCenter`
//! banners posted while the app is backgrounded (a background task keeps the
//! socket alive briefly after the user leaves). Elsewhere they are logged.

use jcode_mobile_core::session::Alert;

#[cfg(target_os = "ios")]
mod imp {
    use super::Alert;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSError, NSString, NSUUID};
    use objc2_ui_kit::{UIApplication, UIApplicationState};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSound, UNUserNotificationCenter,
    };
    use std::cell::Cell;

    thread_local! {
        static BG_TASK: Cell<Option<usize>> = const { Cell::new(None) };
    }

    pub fn request_permission() {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let done = RcBlock::new(|granted: Bool, _err: *mut NSError| {
            log::info!("notifications: permission granted={}", granted.as_bool());
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
            &done,
        );
    }

    fn in_foreground() -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        UIApplication::sharedApplication(mtm).applicationState() == UIApplicationState::Active
    }

    pub fn post(alert: &Alert) {
        if in_foreground() {
            return; // the in-app UI already shows it
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&alert.title));
        content.setBody(&NSString::from_str(&alert.body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let id = NSUUID::UUID().UUIDString();
        let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&req, None);
    }

    /// Ask iOS for extra runtime so an in-flight turn can finish (and notify)
    /// after the user switches away. Call when work is in progress.
    pub fn keep_alive(busy: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = UIApplication::sharedApplication(mtm);
        BG_TASK.with(|t| match (busy, t.get()) {
            (true, None) => {
                let expire = RcBlock::new(|| {
                    if let Some(mtm) = MainThreadMarker::new() {
                        BG_TASK.with(|t| {
                            if let Some(id) = t.take() {
                                UIApplication::sharedApplication(mtm).endBackgroundTask(id);
                            }
                        });
                    }
                });
                t.set(Some(app.beginBackgroundTaskWithExpirationHandler(Some(&expire))));
            }
            (false, Some(id)) => {
                t.set(None);
                app.endBackgroundTask(id);
            }
            _ => {}
        });
    }
}

#[cfg(not(target_os = "ios"))]
mod imp {
    use super::Alert;
    pub fn request_permission() {}
    pub fn post(alert: &Alert) {
        log::info!("notification: {}: {}", alert.title, alert.body);
    }
    pub fn keep_alive(_busy: bool) {}
}

pub use imp::{keep_alive, post, request_permission};
