//! jcode mobile. On Linux/macOS this opens an iPhone-sized preview window
//! (390x844 points, iPhone 15-class safe areas) that talks to a real server.

mod app;
mod scroll_follow;
mod donut;
mod donut_field;
mod sim_keyboard;
mod swipe;
mod system;
mod tabs;
mod markdown;
mod notify;
mod text_input;
mod text_selection;
mod theme;
mod tool_summary;
mod labels;
mod ios_ext;
mod haptics;
mod gestures;
mod share;
mod voice_capture;

use std::borrow::Cow;

use gpui::{App, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};

use app::{MobileApp, SafeArea};

fn load_fonts(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf")),
    ];
    cx.text_system().add_fonts(fonts).expect("load bundled fonts");
}

/// Bundled SVG icons (shared with jcode-desktop's `assets/icons`).
struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/cloud.svg" => Some(Cow::Borrowed(include_bytes!("../assets/icons/cloud.svg"))),
            _ => None,
        })
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(vec!["icons/cloud.svg".into()])
    }
}

fn main() {
    // First thing: capture panics to disk, upload last run's report in the background.
    let crash_path = jcode_mobile_core::crash::default_path();
    jcode_mobile_core::crash::install_hook(crash_path.clone());
    std::thread::spawn(move || {
        let account = jcode_mobile_core::cloud::AccountStore::default_location().load();
        if let Err(e) = jcode_mobile_core::crash::upload_pending(&crash_path, account.as_ref()) {
            log::warn!("jcode-mobile: crash upload failed: {e}");
        }
    });
    let application = gpui_platform::application().with_assets(Assets);
    // jcode:// deep links (share extension, Shortcuts, Live Activity taps).
    application.on_open_urls(ios_ext::handle_urls);
    application.run(|cx: &mut App| {
        log::info!("jcode-mobile: launched");
        load_fonts(cx);
        log::info!("jcode-mobile: fonts loaded");
        app::init(cx);
        // JCODE_MOBILE_SIZE=WxH overrides the device (e.g. 375x667 for an SE).
        let (w, h) = std::env::var("JCODE_MOBILE_SIZE")
            .ok()
            .and_then(|s| {
                let (a, b) = s.split_once('x')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            })
            .unwrap_or((390.0f32, 844.0f32));
        let safe = if h >= 800.0 { SafeArea { top: 47.0, bottom: 34.0 } } else { SafeArea { top: 20.0, bottom: 0.0 } };
        let pad = if std::env::var_os("JCODE_MOBILE_FRAMELESS").is_some() { 0. } else { 40. };
        let bounds = Bounds::centered(None, size(px(w + pad), px(h + pad)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("jcode-mobile".into()),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| {
                // Desktop preview: the mouse acts as a finger, so drags go
                // through gpui's touch recognizer exactly like on the phone
                // (touch-phase pans, taps, long press). JCODE_MOBILE_MOUSE=1
                // restores plain mouse input.
                if !cfg!(target_os = "ios") && std::env::var_os("JCODE_MOBILE_MOUSE").is_none() {
                    window.set_emulate_touch_from_mouse(true);
                }
                let mut app = MobileApp::new(safe, window, cx);
                if !cfg!(target_os = "ios") {
                    app.device = Some((w, h));
                }
                app
            }),
        )
        .expect("open window");
        // Dev hook: JCODE_MOBILE_OPEN_URL=jcode://new?text=hi simulates a deep link.
        if let Ok(url) = std::env::var("JCODE_MOBILE_OPEN_URL") {
            ios_ext::handle_urls(vec![url]);
        }
        log::info!("jcode-mobile: window opened");
        cx.activate(true);
    });
}
