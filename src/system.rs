//! iOS system integration: Dynamic Type, Reduce Motion, light/dark
//! appearance, and Spotlight indexing of session titles.
//!
//! Settings are read from UIKit on iOS and refreshed whenever the app becomes
//! active. On Linux the preview simulates them with env vars:
//! `JCODE_MOBILE_TEXT_SCALE=1.3`, `JCODE_MOBILE_REDUCE_MOTION=1`,
//! `JCODE_MOBILE_APPEARANCE=light|dark`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use gpui::{App, Pixels, px};

use crate::theme::Theme;

static TEXT_SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);
static LIGHT: AtomicBool = AtomicBool::new(false);
/// Session ids tapped in Spotlight, waiting for the app to open them.
static SPOTLIGHT_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Dynamic Type multiplier (1.0 at the default "Large" size).
pub fn text_scale() -> f32 {
    f32::from_bits(TEXT_SCALE.load(Ordering::Relaxed))
}

/// A font size scaled by Dynamic Type.
pub fn fs(size: f32) -> Pixels {
    px((size * text_scale()).round_ties_even_half())
}

trait HalfRound {
    fn round_ties_even_half(self) -> f32;
}
impl HalfRound for f32 {
    /// Round to the nearest half point so glyphs stay crisp.
    fn round_ties_even_half(self) -> f32 {
        (self * 2.).round() / 2.
    }
}

pub fn reduce_motion() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

/// UIContentSizeCategory to a body-text multiplier (Apple's body style goes
/// 14 / 15 / 16 / 17 / 19 / 21 / 23 pt, then the accessibility sizes).
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub fn scale_for_category(category: &str) -> f32 {
    let body = match category.trim_start_matches("UICTContentSizeCategory") {
        "XS" => 14.,
        "S" => 15.,
        "M" => 16.,
        "L" => 17.,
        "XL" => 19.,
        "XXL" => 21.,
        "XXXL" => 23.,
        "AccessibilityM" => 28.,
        "AccessibilityL" => 33.,
        "AccessibilityXL" => 40.,
        "AccessibilityXXL" => 47.,
        "AccessibilityXXXL" => 53.,
        _ => 17.,
    };
    // Cap growth: the phone layout is dense and monospace; past ~1.6x lines
    // stop fitting. Accessibility sizes still grow noticeably.
    (body / 17.0f32).clamp(0.82, 1.6)
}

/// Read current settings and install the matching theme. Returns true when
/// anything changed. Call at startup and whenever the app becomes active.
pub fn refresh(cx: &mut App) -> bool {
    let (scale, reduce, light) = imp::read();
    let changed = scale != text_scale() || reduce != reduce_motion() || light != is_light();
    TEXT_SCALE.store(scale.to_bits(), Ordering::Relaxed);
    REDUCE_MOTION.store(reduce, Ordering::Relaxed);
    LIGHT.store(light, Ordering::Relaxed);
    if changed || !cx.has_global::<Theme>() {
        cx.set_global(if light { Theme::light_neutral() } else { Theme::pure_black() });
    }
    changed
}

/// Install observers (iOS: app-became-active and Spotlight continuation).
pub fn install(cx: &mut App) {
    refresh(cx);
    imp::install();
}

/// Session ids the user tapped in Spotlight since the last call.
pub fn take_spotlight_opens() -> Vec<String> {
    std::mem::take(&mut *SPOTLIGHT_OPEN.lock().unwrap())
}

/// Index session titles in Spotlight (replaces the previous set).
pub fn index_sessions(sessions: &[(String, String, Option<String>)]) {
    use std::hash::{Hash, Hasher};
    static LAST: Mutex<u64> = Mutex::new(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sessions.hash(&mut h);
    let hash = h.finish();
    let mut last = LAST.lock().unwrap();
    if *last == hash {
        return;
    }
    *last = hash;
    imp::index(sessions);
}

pub fn clear_index() {
    imp::clear_index();
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn read() -> (f32, bool, bool) {
        let scale = std::env::var("JCODE_MOBILE_TEXT_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .map(|s| s.clamp(0.5, 3.0))
            .unwrap_or(1.0);
        let reduce = std::env::var("JCODE_MOBILE_REDUCE_MOTION").is_ok_and(|v| v != "0");
        let light = std::env::var("JCODE_MOBILE_APPEARANCE").is_ok_and(|v| v == "light");
        (scale, reduce, light)
    }
    pub fn install() {}
    pub fn index(sessions: &[(String, String, Option<String>)]) {
        log::debug!("spotlight: would index {} sessions", sessions.len());
    }
    pub fn clear_index() {}
}

#[cfg(target_os = "ios")]
mod imp {
    use super::SPOTLIGHT_OPEN;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
    use objc2::{msg_send, sel};
    use objc2_foundation::{NSArray, NSNotificationCenter, NSString};

    /// CSSearchableItemActionType: the userActivity type for Spotlight taps.
    const SPOTLIGHT_ACTIVITY: &str = "com.apple.corespotlightitem";
    /// CSSearchableItemActivityIdentifier: userInfo key holding the item id.
    const SPOTLIGHT_ID_KEY: &str = "kCSSearchableItemActivityIdentifier";
    const DOMAIN: &str = "sh.jcode.mobile.sessions";

    #[link(name = "CoreSpotlight", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" {
        fn UIAccessibilityIsReduceMotionEnabled() -> bool;
    }

    pub fn read() -> (f32, bool, bool) {
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(UIApplication), sharedApplication];
            let scale = if app.is_null() {
                1.0
            } else {
                let cat: *mut NSString = msg_send![app, preferredContentSizeCategory];
                if cat.is_null() { 1.0 } else { super::scale_for_category(&(*cat).to_string()) }
            };
            let reduce = UIAccessibilityIsReduceMotionEnabled();
            // UITraitCollection.currentTraitCollection.userInterfaceStyle: 2 = dark.
            let traits: *mut AnyObject = msg_send![objc2::class!(UITraitCollection), currentTraitCollection];
            let style: isize = if traits.is_null() { 2 } else { msg_send![traits, userInterfaceStyle] };
            (scale, reduce, style == 1)
        }
    }

    pub fn install() {
        // Refresh on foreground: Settings changes land while we are inactive.
        let block = RcBlock::new(|_n: std::ptr::NonNull<objc2_foundation::NSNotification>| {
            super::NEEDS_REFRESH.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        unsafe {
            for name in [
                "UIApplicationDidBecomeActiveNotification",
                "UIContentSizeCategoryDidChangeNotification",
                "UIAccessibilityReduceMotionStatusDidChangeNotification",
            ] {
                let token = NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(name)),
                    None,
                    None,
                    &block,
                );
                std::mem::forget(token);
            }
        }
        std::mem::forget(block);
        add_continue_user_activity();
    }

    /// GPUI's app delegate does not implement
    /// `application:continueUserActivity:restorationHandler:`, so graft it on.
    fn add_continue_user_activity() {
        unsafe extern "C-unwind" fn continue_activity(
            _this: *mut AnyObject,
            _sel: Sel,
            _app: *mut AnyObject,
            activity: *mut AnyObject,
            _restore: *mut AnyObject,
        ) -> Bool {
            unsafe {
                if activity.is_null() {
                    return Bool::NO;
                }
                let kind: *mut NSString = msg_send![activity, activityType];
                if kind.is_null() || (*kind).to_string() != SPOTLIGHT_ACTIVITY {
                    return Bool::NO;
                }
                let info: *mut AnyObject = msg_send![activity, userInfo];
                if info.is_null() {
                    return Bool::NO;
                }
                let key = NSString::from_str(SPOTLIGHT_ID_KEY);
                let id: *mut NSString = msg_send![info, objectForKey: &*key];
                if id.is_null() {
                    return Bool::NO;
                }
                SPOTLIGHT_OPEN.lock().unwrap().push((*id).to_string());
                super::NEEDS_REFRESH.store(true, std::sync::atomic::Ordering::Relaxed);
                Bool::YES
            }
        }
        let Some(class) = AnyClass::get(c"GPUIAppDelegate") else {
            log::warn!("spotlight: GPUIAppDelegate not found");
            return;
        };
        let imp: unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject) -> Bool =
            continue_activity;
        unsafe {
            let ok = objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(application:continueUserActivity:restorationHandler:),
                std::mem::transmute::<_, objc2::runtime::Imp>(imp),
                c"B@:@@@?".as_ptr(),
            );
            if !ok.as_bool() {
                log::warn!("spotlight: continueUserActivity already defined");
            }
        }
    }

    fn index_class() -> Option<&'static AnyClass> {
        AnyClass::get(c"CSSearchableIndex")
    }

    pub fn index(sessions: &[(String, String, Option<String>)]) {
        let Some(index_cls) = index_class() else { return };
        unsafe {
            let mut items: Vec<Retained<AnyObject>> = Vec::with_capacity(sessions.len());
            for (id, title, dir) in sessions {
                let content_type = NSString::from_str("public.text");
                let attrs: *mut AnyObject = msg_send![objc2::class!(CSSearchableItemAttributeSet), alloc];
                let attrs: *mut AnyObject = msg_send![attrs, initWithItemContentType: &*content_type];
                let _: () = msg_send![attrs, setTitle: &*NSString::from_str(title)];
                let desc = match dir {
                    Some(d) => format!("Jcode session in {d}"),
                    None => "Jcode session".to_string(),
                };
                let _: () = msg_send![attrs, setContentDescription: &*NSString::from_str(&desc)];
                let item: *mut AnyObject = msg_send![objc2::class!(CSSearchableItem), alloc];
                let item: *mut AnyObject = msg_send![
                    item,
                    initWithUniqueIdentifier: &*NSString::from_str(id),
                    domainIdentifier: &*NSString::from_str(DOMAIN),
                    attributeSet: attrs
                ];
                let _: () = msg_send![attrs, release];
                if let Some(item) = Retained::from_raw(item) {
                    items.push(item);
                }
            }
            let array = NSArray::from_retained_slice(&items);
            let index: *mut AnyObject = msg_send![index_cls, defaultSearchableIndex];
            let domains = NSArray::from_retained_slice(&[NSString::from_str(DOMAIN)]);
            // Replace the domain wholesale so deleted sessions disappear.
            let done = RcBlock::new(move |_err: *mut AnyObject| {
                let index: *mut AnyObject = msg_send![objc2::class!(CSSearchableIndex), defaultSearchableIndex];
                let _: () = msg_send![index, indexSearchableItems: &*array, completionHandler: std::ptr::null::<AnyObject>()];
            });
            let _: () = msg_send![index, deleteSearchableItemsWithDomainIdentifiers: &*domains, completionHandler: &*done];
        }
    }

    pub fn clear_index() {
        let Some(index_cls) = index_class() else { return };
        unsafe {
            let index: *mut AnyObject = msg_send![index_cls, defaultSearchableIndex];
            let domains = NSArray::from_retained_slice(&[NSString::from_str(DOMAIN)]);
            let _: () = msg_send![index, deleteSearchableItemsWithDomainIdentifiers: &*domains, completionHandler: std::ptr::null::<AnyObject>()];
        }
    }
}

/// Set from UIKit callbacks; the app polls it each frame (cheap atomic).
pub static NEEDS_REFRESH: AtomicBool = AtomicBool::new(false);

/// Poll from render: re-read settings after a UIKit notification. Returns
/// true when the app should redraw with new metrics or palette.
pub fn poll(cx: &mut App) -> bool {
    if NEEDS_REFRESH.swap(false, Ordering::Relaxed) {
        refresh(cx);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_scale_monotonically() {
        let cats = ["XS", "S", "M", "L", "XL", "XXL", "XXXL", "AccessibilityM", "AccessibilityXXXL"];
        let scales: Vec<f32> = cats.iter().map(|c| scale_for_category(&format!("UICTContentSizeCategory{c}"))).collect();
        assert!(scales.windows(2).all(|w| w[0] <= w[1]), "{scales:?}");
        assert_eq!(scale_for_category("UICTContentSizeCategoryL"), 1.0);
        assert_eq!(scale_for_category("garbage"), 1.0);
        assert_eq!(scale_for_category("UICTContentSizeCategoryAccessibilityXXXL"), 1.6);
    }

    #[test]
    fn fs_rounds_to_half_points() {
        TEXT_SCALE.store(1.3f32.to_bits(), Ordering::Relaxed);
        assert_eq!(fs(13.), px(17.));
        TEXT_SCALE.store(1.0f32.to_bits(), Ordering::Relaxed);
        assert_eq!(fs(13.5), px(13.5));
    }
}
