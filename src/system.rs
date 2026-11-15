//! iOS system integration: Dynamic Type, Reduce Motion, light/dark
//! appearance, and Spotlight indexing of session titles. Android reads its
//! Configuration font scale/night mode and animation setting, and indexes
//! session titles as dynamic launcher shortcuts.
//!
//! Settings are read from UIKit on iOS and refreshed whenever the app becomes
//! active. On Linux the preview simulates them with env vars:
//! `JCODE_MOBILE_TEXT_SCALE=1.3`, `JCODE_MOBILE_REDUCE_MOTION=1`,
//! `JCODE_MOBILE_APPEARANCE=light|dark`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(not(test))]
use std::sync::atomic::AtomicU32;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use gpui::{App, Pixels, px};

use crate::theme::Theme;

#[cfg(not(test))]
static TEXT_SCALE: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0f32
#[cfg(not(test))]
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);
#[cfg(not(test))]
static LIGHT: AtomicBool = AtomicBool::new(false);

// Each GPUI test owns its UI thread. Process-wide settings allow another
// window's initialization to change metrics halfway through layout.
#[cfg(test)]
thread_local! {
    static TEST_SETTINGS: std::cell::Cell<(f32, bool, bool)> = const { std::cell::Cell::new((1., false, false)) };
    static TEST_OVERRIDE: std::cell::Cell<Option<(f32, bool, bool)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) struct SettingsFixture(Option<(f32, bool, bool)>, (f32, bool, bool));

#[cfg(test)]
pub(crate) fn settings_fixture(scale: f32, reduce: bool, light: bool) -> SettingsFixture {
    let settings = (scale, reduce, light);
    SettingsFixture(TEST_OVERRIDE.with(|value| value.replace(Some(settings))),
        TEST_SETTINGS.with(|value| value.replace(settings)))
}

#[cfg(test)]
impl Drop for SettingsFixture {
    fn drop(&mut self) {
        TEST_OVERRIDE.with(|value| value.set(self.0));
        TEST_SETTINGS.with(|value| value.set(self.1));
    }
}
/// Session ids tapped in Spotlight, waiting for the app to open them.
static SPOTLIGHT_OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Dynamic Type multiplier (1.0 at the default "Large" size).
pub fn text_scale() -> f32 {
    #[cfg(test)]
    { return TEST_SETTINGS.with(|value| value.get().0); }
    #[cfg(not(test))]
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
    #[cfg(test)]
    { return TEST_SETTINGS.with(|value| value.get().1); }
    #[cfg(not(test))]
    REDUCE_MOTION.load(Ordering::Relaxed)
}

pub fn is_light() -> bool {
    #[cfg(test)]
    { return TEST_SETTINGS.with(|value| value.get().2); }
    #[cfg(not(test))]
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
    #[cfg(not(test))]
    let (scale, reduce, light) = imp::read();
    #[cfg(test)]
    let (scale, reduce, light) = TEST_OVERRIDE.with(|value| value.get().unwrap_or((1., false, false)));
    let changed = scale != text_scale() || reduce != reduce_motion() || light != is_light();
    #[cfg(not(test))]
    {
        TEXT_SCALE.store(scale.to_bits(), Ordering::Relaxed);
        REDUCE_MOTION.store(reduce, Ordering::Relaxed);
        LIGHT.store(light, Ordering::Relaxed);
    }
    #[cfg(test)]
    TEST_SETTINGS.with(|value| value.set((scale, reduce, light)));
    if changed || !cx.has_global::<Theme>() {
        cx.set_global(if light { Theme::light_neutral() } else { Theme::pure_black() });
    }
    changed
}

/// Install observers (iOS: app-became-active and Spotlight continuation).
pub fn install(cx: &mut App) {
    refresh(cx);
    #[cfg(target_os = "android")]
    install_refresh_channel(cx);
    imp::install();
}

/// Session ids the user tapped in Spotlight since the last call.
pub fn take_spotlight_opens() -> Vec<String> {
    std::mem::take(&mut *SPOTLIGHT_OPEN.lock().unwrap())
}

pub(crate) const PLATFORM_RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Default)]
struct IndexCache {
    applied: Option<u64>,
    retry_at: Option<Instant>,
    pending_clear: bool,
}

impl IndexCache {
    fn prepare<T>(&self, hash: u64, now: Instant, build: impl FnOnce() -> T) -> Option<T> {
        (self.applied != Some(hash) && !self.pending_clear
            && self.retry_at.is_none_or(|retry| now >= retry)).then(build)
    }

    fn complete(&mut self, hash: u64, accepted: bool, now: Instant) {
        self.applied = accepted.then_some(hash);
        self.retry_at = (!accepted).then_some(now + PLATFORM_RETRY_DELAY);
    }

    fn clear_result(&mut self, accepted: bool, now: Instant) {
        self.applied = None;
        self.pending_clear = !accepted;
        self.retry_at = (!accepted).then_some(now + PLATFORM_RETRY_DELAY);
    }
}

static INDEX_CACHE: Mutex<IndexCache> = Mutex::new(IndexCache {
    applied: None, retry_at: None, pending_clear: false,
});
// In-flight delete completions must not restore records after sign-out or a
// newer account/list replacement.
static INDEX_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn index_fingerprint<'a>(owner: impl Hash, sessions: impl IntoIterator<Item = (&'a str, Option<&'a str>, Option<&'a str>)>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    owner.hash(&mut h);
    let mut count = 0usize;
    for session in sessions {
        session.hash(&mut h);
        count += 1;
    }
    count.hash(&mut h);
    h.finish()
}

/// Hash borrowed source metadata each frame, allocating titles and records only
/// when it changes. `owner` includes entity identity and authentication epoch.
/// Empty lists are replacements too, so deleted sessions cannot remain indexed.
pub fn index_sessions<'a>(
    owner: impl Hash,
    sessions: impl IntoIterator<Item = (&'a str, Option<&'a str>, Option<&'a str>)>,
    build: impl FnOnce() -> Vec<(String, String, Option<String>)>,
) {
    let hash = index_fingerprint(owner, sessions);
    let mut cache = INDEX_CACHE.lock().unwrap();
    if let Some(sessions) = cache.prepare(hash, Instant::now(), build) {
        INDEX_GENERATION.fetch_add(1, Ordering::Relaxed);
        let accepted = imp::index(&sessions);
        cache.complete(hash, accepted, Instant::now());
    }
}

pub fn clear_index() {
    let mut cache = INDEX_CACHE.lock().unwrap();
    INDEX_GENERATION.fetch_add(1, Ordering::Relaxed);
    let accepted = imp::clear_index();
    cache.clear_result(accepted, Instant::now());
}

#[cfg(target_os = "android")]
mod imp {
    pub fn read() -> (f32, bool, bool) { crate::android_services::settings() }
    pub fn install() {}
    pub fn index(sessions: &[(String, String, Option<String>)]) -> bool {
        let json = serde_json::to_string(sessions).unwrap_or_default();
        let accepted = crate::android_services::call_bool("indexSessions", &json);
        if !accepted {
            log::debug!("Android launcher session shortcuts unavailable");
        }
        accepted
    }
    pub fn clear_index() -> bool { crate::android_services::call_bool("indexSessions", "[]") }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
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
    pub fn index(sessions: &[(String, String, Option<String>)]) -> bool {
        log::debug!("spotlight: would index {} sessions", sessions.len());
        true
    }
    pub fn clear_index() -> bool { true }
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

    type ContinueFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject, *mut AnyObject) -> Bool;
    /// GPUI's own `continueUserActivity` (deep links), if it has one.
    static PREVIOUS: std::sync::OnceLock<Option<ContinueFn>> = std::sync::OnceLock::new();

    /// Handle Spotlight taps in `application:continueUserActivity:restorationHandler:`
    /// on GPUI's app delegate, chaining to any implementation it already has.
    fn add_continue_user_activity() {
        unsafe extern "C-unwind" fn continue_activity(
            this: *mut AnyObject,
            sel: Sel,
            app: *mut AnyObject,
            activity: *mut AnyObject,
            restore: *mut AnyObject,
        ) -> Bool {
            unsafe {
                if let Some(id) = spotlight_id(activity) {
                    SPOTLIGHT_OPEN.lock().unwrap().push(id);
                    super::NEEDS_REFRESH.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Bool::YES;
                }
                match PREVIOUS.get().copied().flatten() {
                    Some(prev) => prev(this, sel, app, activity, restore),
                    None => Bool::NO,
                }
            }
        }
        unsafe fn spotlight_id(activity: *mut AnyObject) -> Option<String> {
            unsafe {
                if activity.is_null() {
                    return None;
                }
                let kind: *mut NSString = msg_send![activity, activityType];
                if kind.is_null() || (*kind).to_string() != SPOTLIGHT_ACTIVITY {
                    return None;
                }
                let info: *mut AnyObject = msg_send![activity, userInfo];
                if info.is_null() {
                    return None;
                }
                let key = NSString::from_str(SPOTLIGHT_ID_KEY);
                let id: *mut NSString = msg_send![info, objectForKey: &*key];
                (!id.is_null()).then(|| (*id).to_string())
            }
        }
        let Some(class) = AnyClass::get(c"GPUIAppDelegate") else {
            log::warn!("spotlight: GPUIAppDelegate not found");
            return;
        };
        let imp: ContinueFn = continue_activity;
        unsafe {
            let prev = objc2::ffi::class_replaceMethod(
                class as *const AnyClass as *mut AnyClass,
                sel!(application:continueUserActivity:restorationHandler:),
                std::mem::transmute::<ContinueFn, objc2::runtime::Imp>(imp),
                c"B@:@@@?".as_ptr(),
            );
            let _ = PREVIOUS.set(prev.map(|p| std::mem::transmute::<objc2::runtime::Imp, ContinueFn>(p)));
        }
    }

    fn index_class() -> Option<&'static AnyClass> {
        AnyClass::get(c"CSSearchableIndex")
    }

    pub fn index(sessions: &[(String, String, Option<String>)]) -> bool {
        let Some(index_cls) = index_class() else { return false };
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
            let generation = super::INDEX_GENERATION.load(std::sync::atomic::Ordering::Relaxed);
            let done = RcBlock::new(move |_err: *mut AnyObject| {
                // Serialize the generation check and submission with clear_index.
                // Otherwise sign-out could land between these two operations.
                let _cache = super::INDEX_CACHE.lock().unwrap();
                if super::INDEX_GENERATION.load(std::sync::atomic::Ordering::Relaxed) != generation {
                    return;
                }
                let index: *mut AnyObject = msg_send![objc2::class!(CSSearchableIndex), defaultSearchableIndex];
                let _: () = msg_send![index, indexSearchableItems: &*array, completionHandler: std::ptr::null::<AnyObject>()];
            });
            let _: () = msg_send![index, deleteSearchableItemsWithDomainIdentifiers: &*domains, completionHandler: &*done];
        }
        // Spotlight's completion remains asynchronous. This acknowledges the
        // submission, not completion of the OS indexing operation.
        true
    }

    pub fn clear_index() -> bool {
        let Some(index_cls) = index_class() else { return false };
        unsafe {
            let index: *mut AnyObject = msg_send![index_cls, defaultSearchableIndex];
            let domains = NSArray::from_retained_slice(&[NSString::from_str(DOMAIN)]);
            let _: () = msg_send![index, deleteSearchableItemsWithDomainIdentifiers: &*domains, completionHandler: std::ptr::null::<AnyObject>()];
        }
        true
    }
}

/// Set from UIKit callbacks; the app polls it each frame (cheap atomic).
pub static NEEDS_REFRESH: AtomicBool = AtomicBool::new(false);

#[cfg(any(target_os = "android", test))]
static REFRESH_TX: Mutex<Option<futures::channel::mpsc::UnboundedSender<()>>> = Mutex::new(None);

/// Java callbacks only enqueue work. Never enter a cache or call back into
/// Java here: the GPUI thread may already be waiting for Android's UI thread.
#[cfg(any(target_os = "android", test))]
pub(crate) fn request_refresh() {
    NEEDS_REFRESH.store(true, Ordering::Relaxed);
    if let Some(tx) = REFRESH_TX.lock().unwrap().as_ref() {
        let _ = tx.unbounded_send(());
    }
}

#[cfg(any(target_os = "android", test))]
fn install_refresh_channel(cx: &mut App) {
    use futures::StreamExt as _;
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    *REFRESH_TX.lock().unwrap() = Some(tx);
    cx.spawn(async move |cx| {
        while rx.next().await.is_some() {
            cx.update(|cx| {
                // Reading settings and applying pending platform work both
                // happen on GPUI's UI thread, not the Activity callback thread.
                if !poll(cx) { refresh(cx); }
                cx.refresh_windows();
            });
        }
    }).detach();
}

struct RetryWake {
    at: Instant,
    _task: gpui::Task<()>,
}
impl gpui::Global for RetryWake {}

/// One delayed wake per App, not a frame loop or one timer per failed call.
/// The next render reads current account data rather than replaying old records.
pub(crate) fn schedule_platform_retry(cx: &mut App) {
    let index_at = INDEX_CACHE.lock().unwrap().retry_at;
    let live_at = crate::ios_ext::live_retry_at();
    let at = index_at.into_iter().chain(live_at).min();
    schedule_retry_wake(at, cx);
}

fn schedule_retry_wake(at: Option<Instant>, cx: &mut App) {
    let Some(at) = at else {
        if cx.has_global::<RetryWake>() { cx.remove_global::<RetryWake>(); }
        return;
    };
    let now = Instant::now();
    if cx.has_global::<RetryWake>() {
        let wake = cx.global::<RetryWake>();
        if wake.at > now && wake.at <= at { return; }
    }
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(at.saturating_duration_since(Instant::now())).await;
        cx.update(|cx| {
            if cx.has_global::<RetryWake>() && cx.global::<RetryWake>().at == at {
                cx.remove_global::<RetryWake>();
                cx.refresh_windows();
            }
        });
    });
    cx.set_global(RetryWake { at, _task: task });
}

/// Poll from render: re-read settings after a UIKit notification. Returns
/// true when the app should redraw with new metrics or palette.
pub fn poll(cx: &mut App) -> bool {
    let requested = NEEDS_REFRESH.swap(false, Ordering::Relaxed);
    {
        let mut cache = INDEX_CACHE.lock().unwrap();
        if requested { cache.retry_at = None; }
        if cache.pending_clear && cache.retry_at.is_none_or(|at| Instant::now() >= at) {
            INDEX_GENERATION.fetch_add(1, Ordering::Relaxed);
            let accepted = imp::clear_index();
            cache.clear_result(accepted, Instant::now());
        }
    }
    crate::ios_ext::retry_live(Instant::now(), requested);
    if requested {
        refresh(cx);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn android_settings_signal_refreshes_on_gpui_thread_without_another_frame(cx: &mut gpui::TestAppContext) {
        let _fixture = settings_fixture(1., false, false);
        cx.update(|cx| {
            refresh(cx);
            install_refresh_channel(cx);
        });
        TEST_OVERRIDE.with(|value| value.set(Some((1.5, true, true))));
        std::thread::spawn(request_refresh).join().unwrap();
        assert_eq!(text_scale(), 1., "the Activity callback must not mutate UI settings directly");
        cx.run_until_parked();
        assert_eq!((text_scale(), reduce_motion(), is_light()), (1.5, true, true));
        REFRESH_TX.lock().unwrap().take();
    }

    #[gpui::test]
    fn platform_retry_uses_one_delayed_wake_and_can_be_cancelled(cx: &mut gpui::TestAppContext) {
        let now = Instant::now();
        cx.update(|cx| {
            schedule_retry_wake(Some(now + PLATFORM_RETRY_DELAY), cx);
            schedule_retry_wake(Some(now + PLATFORM_RETRY_DELAY + Duration::from_secs(1)), cx);
            assert_eq!(cx.global::<RetryWake>().at, now + PLATFORM_RETRY_DELAY);
        });
        cx.run_until_parked();
        cx.executor().advance_clock(PLATFORM_RETRY_DELAY);
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!cx.has_global::<RetryWake>(), "retry must wake even without another platform event");
            schedule_retry_wake(Some(Instant::now() + PLATFORM_RETRY_DELAY), cx);
            schedule_retry_wake(None, cx);
            assert!(!cx.has_global::<RetryWake>());
        });
    }

    #[test]
    fn settings_fixtures_are_thread_local_and_restore_on_unwind() {
        let fixture = settings_fixture(2., true, true);
        std::thread::spawn(|| {
            assert_eq!((text_scale(), reduce_motion(), is_light()), (1., false, false));
            let _fixture = settings_fixture(1.6, false, false);
            assert_eq!(text_scale(), 1.6);
        }).join().unwrap();
        assert_eq!((text_scale(), reduce_motion(), is_light()), (2., true, true));
        let _ = std::panic::catch_unwind(|| {
            let _nested = settings_fixture(1.3, false, false);
            panic!("fixture unwind");
        });
        assert_eq!((text_scale(), reduce_motion(), is_light()), (2., true, true));
        drop(fixture);
        assert_eq!((text_scale(), reduce_motion(), is_light()), (1., false, false));
    }

    #[test]
    fn spotlight_unchanged_metadata_does_not_build_records() {
        let mut cache = IndexCache::default();
        let now = Instant::now();
        let records = [("session_one_123", Some("Title"), Some("/project"))];
        let hash = index_fingerprint((1u64, 1u64), records);
        assert_eq!(cache.prepare(hash, now, || 1), Some(1));
        assert_eq!(cache.applied, None, "preparation is not successful publication");
        cache.complete(hash, true, now);
        for _ in 0..100 {
            assert_eq!(cache.prepare(hash, now, || panic!("unchanged metadata rebuilt")), None::<()>);
        }
        // Clear must permit exactly the same list to be indexed after login.
        cache.clear_result(true, now);
        assert_eq!(cache.prepare(hash, now, || 2), Some(2));
        // Zero is a valid fingerprint, not an uninitialized sentinel.
        assert_eq!(IndexCache::default().prepare(0, now, || ()), Some(()));
    }

    #[test]
    fn spotlight_invalidates_for_all_metadata_and_account_changes() {
        let owner = (1u64, 1u64);
        let records = [("id", Some("Title"), Some("/project"))];
        let original = index_fingerprint(owner, records);
        for changed in [
            [("other-id", Some("Title"), Some("/project"))],
            [("id", Some("Renamed"), Some("/project"))],
            [("id", None, Some("/project"))],
            [("id", Some("Title"), Some("/other-project"))],
            [("id", Some("Title"), None)],
        ] {
            assert_ne!(original, index_fingerprint(owner, changed));
        }
        assert_ne!(original, index_fingerprint((2u64, 1u64), records));
        assert_ne!(original, index_fingerprint((1u64, 2u64), records));
        let empty = index_fingerprint(owner, []);
        let now = Instant::now();
        let mut cache = IndexCache::default();
        cache.complete(original, true, now);
        assert_eq!(cache.prepare(empty, now, Vec::<()>::new), Some(vec![]));
        cache.complete(empty, true, now);
        assert_eq!(cache.prepare(empty, now, || panic!("empty replacement repeated")), None::<()>);
        assert_ne!(empty, index_fingerprint((1u64, 2u64), []));
    }

    #[test]
    fn shortcut_failures_retry_unchanged_metadata_without_frame_spam() {
        let now = Instant::now();
        let later = now + PLATFORM_RETRY_DELAY;
        let mut cache = IndexCache::default();
        assert_eq!(cache.prepare(1, now, || "records"), Some("records"));
        cache.complete(1, false, now);
        assert_eq!(cache.applied, None);
        for hash in [1, 2] {
            assert_eq!(cache.prepare(hash, now, || panic!("failed platform called every frame")), None::<()>);
        }
        assert_eq!(cache.prepare(1, later, || "same records"), Some("same records"));
        cache.complete(1, true, later);
        assert_eq!(cache.prepare(1, later, || panic!("successful metadata rebuilt")), None::<()>);
        assert_eq!(cache.prepare(2, later, || "new records"), Some("new records"));
    }

    #[test]
    fn shortcut_clear_bypasses_failure_and_failed_clear_blocks_private_republication() {
        let now = Instant::now();
        let mut cache = IndexCache::default();
        cache.complete(1, false, now);
        // clear_index always attempts the platform cancellation immediately,
        // regardless of this earlier failed publication's retry deadline.
        cache.clear_result(false, now);
        assert!(cache.pending_clear);
        assert_eq!(cache.prepare(1, now + PLATFORM_RETRY_DELAY, || panic!("old account republished before clear")), None::<()>);
        cache.clear_result(true, now + PLATFORM_RETRY_DELAY);
        assert!(!cache.pending_clear);
        assert_eq!(cache.prepare(1, now + PLATFORM_RETRY_DELAY, || "same list after login"), Some("same list after login"));
    }

    #[test]
    #[ignore = "manual Spotlight preparation timing comparison"]
    fn benchmark_spotlight_unchanged_preparation() {
        use std::hint::black_box;
        use std::time::Instant;
        let records: Vec<_> = (0..1000).map(|n| (
            format!("session_project_{n}"), format!("Project session {n}"), Some("/workspace/project".to_owned()),
        )).collect();
        let start = Instant::now();
        for _ in 0..1000 {
            let cloned = black_box(&records).clone();
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            cloned.hash(&mut hash);
            black_box(hash.finish());
        }
        let cloned = start.elapsed();
        let start = Instant::now();
        let mut cache = IndexCache::default();
        let now = Instant::now();
        for _ in 0..1000 {
            let hash = index_fingerprint((1u64, 1u64), black_box(&records).iter()
                .map(|(id, title, dir)| (id.as_str(), Some(title.as_str()), dir.as_deref())));
            black_box(cache.prepare(hash, now, || records.clone()));
            cache.complete(hash, true, now);
        }
        eprintln!("1000 sessions x 1000 frames: cloned={cloned:?}, borrowed={:?}", start.elapsed());
    }

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
        let fixture = settings_fixture(1.3, false, false);
        assert_eq!(fs(13.), px(17.));
        drop(fixture);
        assert_eq!(fs(13.5), px(13.5));
    }
}
