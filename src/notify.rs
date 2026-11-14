//! Notifications.
//!
//! Remote (APNs) pushes are the real channel: the jcode server asks
//! api.jcode.sh to push when a turn finishes, errors, needs input, or wakes,
//! so they arrive with the phone locked or the app killed (see docs/PUSH.md).
//! This module registers for them, installs the `UNUserNotificationCenter`
//! delegate (foreground presentation, taps, inline Reply), registers the
//! actionable categories, and manages the badge.
//!
//! Local banners (`post`) remain as a fallback for the few seconds iOS keeps
//! the socket alive after the user leaves, and are skipped once APNs is live
//! so the user never gets the same news twice.

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use jcode_mobile_core::session::Alert;
use std::sync::Mutex;

/// What the notification layer needs the app to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushEvent {
    /// Fresh APNs token (hex). Register it with the account.
    Token(String),
    /// The user tapped a notification (or its Open action).
    Open { session_id: String },
    /// The user answered from the notification's inline Reply field.
    Reply { session_id: String, text: String },
    /// The app came to the foreground.
    Foreground,
}

static TX: Mutex<Option<UnboundedSender<PushEvent>>> = Mutex::new(None);
/// Session on screen. Pushes for it are not bannered while foregrounded.
static VISIBLE: Mutex<Option<String>> = Mutex::new(None);
/// True once an APNs token is registered, so local fallbacks stand down.
static REMOTE_LIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn emit(event: PushEvent) {
    if let Some(tx) = TX.lock().unwrap().as_ref() {
        let _ = tx.unbounded_send(event);
    }
}

/// Install handlers. Call once from the main thread at startup.
pub fn install() -> UnboundedReceiver<PushEvent> {
    let (tx, rx) = unbounded();
    *TX.lock().unwrap() = Some(tx);
    imp::install();
    rx
}

pub fn set_visible_session(id: Option<String>) {
    *VISIBLE.lock().unwrap() = id;
}

pub fn mark_remote_live() {
    REMOTE_LIVE.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn remote_live() -> bool {
    REMOTE_LIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// APNs environment for this build's entitlement.
pub fn push_environment() -> jcode_mobile_core::cloud::PushEnvironment {
    if cfg!(debug_assertions) {
        jcode_mobile_core::cloud::PushEnvironment::Sandbox
    } else {
        jcode_mobile_core::cloud::PushEnvironment::Production
    }
}

pub const BUNDLE_ID: &str = "sh.jcode.mobile.rust";

#[cfg(target_os = "ios")]
mod imp {
    use super::{Alert, PushEvent, emit};
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread, MainThreadMarker, define_class, msg_send};
    use objc2_foundation::{NSArray, NSDictionary, NSError, NSNotificationCenter, NSSet, NSString, NSUUID};
    use objc2_ui_kit::{UIApplication, UIApplicationDidBecomeActiveNotification, UIApplicationState};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
        UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNNotificationSound, UNTextInputNotificationResponse, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use std::cell::{Cell, RefCell};

    thread_local! {
        static BG_TASK: Cell<Option<usize>> = const { Cell::new(None) };
        static KEEP: RefCell<Vec<Retained<NSObject>>> = const { RefCell::new(Vec::new()) };
        static DELEGATE: RefCell<Option<Retained<PushDelegate>>> = const { RefCell::new(None) };
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "JcodePushDelegate"]
        struct PushDelegate;

        unsafe impl NSObjectProtocol for PushDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for PushDelegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                notification: &UNNotification,
                completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                // Foreground: banner unless it is about the session on screen.
                let session = session_of(notification);
                let visible = super::VISIBLE.lock().unwrap().clone();
                let opts = if session.is_some() && session == visible {
                    UNNotificationPresentationOptions::empty()
                } else {
                    UNNotificationPresentationOptions::Banner
                        | UNNotificationPresentationOptions::List
                        | UNNotificationPresentationOptions::Sound
                };
                completion.call((opts,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion: &block2::DynBlock<dyn Fn()>,
            ) {
                let session = session_of(&response.notification());
                let action = response.actionIdentifier().to_string();
                if let Some(session_id) = session {
                    if action == "REPLY" {
                        let text = response
                            .downcast_ref::<UNTextInputNotificationResponse>()
                            .map(|r| r.userText().to_string())
                            .unwrap_or_default();
                        if !text.trim().is_empty() {
                            emit(PushEvent::Reply { session_id, text });
                        }
                    } else if action == "CONTINUE" {
                        emit(PushEvent::Reply { session_id, text: "Continue".into() });
                    } else if action != "com.apple.UNNotificationDismissActionIdentifier" {
                        emit(PushEvent::Open { session_id });
                    }
                }
                completion.call(());
            }
        }
    );

    fn session_of(notification: &UNNotification) -> Option<String> {
        let info: Retained<NSDictionary> = notification.request().content().userInfo();
        let key = NSString::from_str("session_id");
        let value: Option<Retained<objc2::runtime::AnyObject>> =
            unsafe { msg_send![&*info, objectForKey: &*key] };
        let value = value?;
        let s = value.downcast::<NSString>().ok()?.to_string();
        (!s.is_empty()).then_some(s)
    }

    fn action(id: &str, title: &str, options: UNNotificationActionOptions) -> Retained<UNNotificationAction> {
        UNNotificationAction::actionWithIdentifier_title_options(
            &NSString::from_str(id),
            &NSString::from_str(title),
            options,
        )
    }

    fn reply(button: &str) -> Retained<UNNotificationAction> {
        objc2_user_notifications::UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
            &NSString::from_str("REPLY"),
            &NSString::from_str(button),
            UNNotificationActionOptions::empty(),
            &NSString::from_str("Send"),
            &NSString::from_str("Message jcode"),
        )
        .into_super()
    }

    fn category(id: &str, reply_title: &str, quick_continue: bool) -> Retained<UNNotificationCategory> {
        let mut list = vec![reply(reply_title)];
        if quick_continue {
            // One tap from the lock screen: carry on without opening the app.
            list.push(action("CONTINUE", "Continue", UNNotificationActionOptions::empty()));
        }
        list.push(action("OPEN", "Open", UNNotificationActionOptions::Foreground));
        let actions = NSArray::from_retained_slice(&list);
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(id),
            &actions,
            &NSArray::new(),
            UNNotificationCategoryOptions::HiddenPreviewsShowTitle,
        )
    }

    pub fn install() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate: Retained<PushDelegate> = unsafe { msg_send![PushDelegate::alloc(), init] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));
        center.setNotificationCategories(&NSSet::from_retained_slice(&[
            category("JCODE_TURN", "Reply", true),
            category("JCODE_INPUT", "Answer", false),
        ]));

        // Foregrounding resets the badge and lets the app refresh state.
        let block = RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            jcode_mobile_core::timing::event("ios foreground", "");
            emit(PushEvent::Foreground);
        });
        let token = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(UIApplicationDidBecomeActiveNotification),
                None,
                None,
                &block,
            )
        };
        KEEP.with(|k| k.borrow_mut().push(unsafe { Retained::cast_unchecked(token) }));

        // Observe lifecycle signals only. Never inspect notification userInfo.
        for (notification, label) in [
            ("UIApplicationDidEnterBackgroundNotification", "ios background"),
            ("UIApplicationDidReceiveMemoryWarningNotification", "ios memory warning"),
        ] {
            let block = RcBlock::new(move |_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                jcode_mobile_core::timing::event(label, "");
            });
            let token = unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(notification)),
                    None,
                    None,
                    &block,
                )
            };
            KEEP.with(|k| k.borrow_mut().push(unsafe { Retained::cast_unchecked(token) }));
        }

        gpui_platform::ios::register_for_remote_notifications(|event| {
            use gpui_platform::ios::RemoteNotificationEvent as E;
            match event {
                E::Token(raw) => {
                    super::mark_remote_live();
                    emit(PushEvent::Token(jcode_mobile_core::cloud::push_token_hex(&raw)));
                }
                E::RegistrationFailed(e) => log::warn!("push: APNs registration failed: {e}"),
                E::Received { .. } => {}
            }
        });
        let _ = mtm;
    }

    pub fn request_permission() {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let done = RcBlock::new(|granted: Bool, _err: *mut NSError| {
            log::info!("notifications: permission granted={}", granted.as_bool());
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Sound
                | UNAuthorizationOptions::Badge
                | UNAuthorizationOptions::ProvidesAppNotificationSettings,
            &done,
        );
    }

    pub fn clear_badge() {
        gpui_platform::ios::set_badge_count(0);
        UNUserNotificationCenter::currentNotificationCenter().removeAllDeliveredNotifications();
    }

    fn in_foreground() -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        UIApplication::sharedApplication(mtm).applicationState() == UIApplicationState::Active
    }

    pub fn post(alert: &Alert) {
        if in_foreground() || super::remote_live() {
            return; // in-app UI shows it, or APNs delivers it
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&alert.title));
        content.setBody(&NSString::from_str(&alert.body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        content.setCategoryIdentifier(&NSString::from_str("JCODE_TURN"));
        if let Some(sid) = &alert.session_id {
            content.setThreadIdentifier(&NSString::from_str(sid));
            let info = NSDictionary::from_retained_objects(
                &[&*NSString::from_str("session_id")],
                &[Retained::into_super(Retained::into_super(NSString::from_str(sid)))],
            );
            let info: Retained<NSDictionary> = unsafe { Retained::cast_unchecked(info) };
            unsafe { content.setUserInfo(&info) };
        }
        let id = NSUUID::UUID().UUIDString();
        let req = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&req, None);
    }

    /// Ask iOS for extra runtime so an in-flight turn can finish after the
    /// user switches away. Call when work is in progress.
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
    pub fn install() {}
    pub fn request_permission() {}
    pub fn clear_badge() {}
    pub fn post(alert: &Alert) {
        log::info!("notification: {}: {}", alert.title, alert.body);
    }
    pub fn keep_alive(_busy: bool) {}
}

pub use imp::{clear_badge, keep_alive, post, request_permission};
