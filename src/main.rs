//! jcode mobile. On Linux/macOS this opens an iPhone-sized preview window
//! (390x844 points, iPhone 15-class safe areas) that talks to a real server.

mod app;
mod donut;
mod donut_field;
mod sim_keyboard;
mod tabs;
mod markdown;
mod notify;
mod text_input;
mod theme;
mod tool_summary;

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

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
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
                let mut app = MobileApp::new(safe, window, cx);
                if !cfg!(target_os = "ios") {
                    app.device = Some((w, h));
                }
                app
            }),
        )
        .expect("open window");
        log::info!("jcode-mobile: window opened");
        cx.activate(true);
    });
}
