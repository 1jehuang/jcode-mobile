//! Design tokens from Jcode Desktop's default "Pure black" palette
//! (jcode-desktop-ui-core/src/theme/palettes.rs), so the phone app and the
//! desktop app read as one product. Desktop uses one monospace face for UI,
//! assistant text and code; we bundle JetBrains Mono for the same look.

use gpui::{App, Global, Hsla, rgb, rgba};

pub const FONT: &str = "JetBrains Mono";

#[derive(Clone, Copy)]
pub struct Theme {
    /// Canvas and pane paper (`BG`, `PANEL_BG`).
    pub background: Hsla,
    /// Recessed chrome: header bar, inactive tabs (`HEADER_BG`).
    pub raised: Hsla,
    /// Prompt cards and pills (`USER_BG`).
    pub surface: Hsla,
    /// Hover/pressed fill and machine chip (`ACCENT_DIM`).
    pub surface_elevated: Hsla,
    pub code_bg: Hsla,
    pub inline_code_bg: Hsla,
    pub border: Hsla,
    /// Composer and focused-tab outline (`PANEL_BORDER_FOCUS`).
    pub border_focus: Hsla,
    pub input_border: Hsla,
    pub accent: Hsla,
    pub text: Hsla,
    /// User prompt and heading ink (`TEXT_USER`, `HEADING`).
    pub text_strong: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,
    pub ok: Hsla,
    pub warning: Hsla,
    pub error: Hsla,
    pub field: Hsla,
    pub selection: Hsla,
}

impl Global for Theme {}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    pub fn pure_black() -> Self {
        Self {
            background: c(0x000000),
            raised: c(0x0a0a0a),
            surface: c(0x121212),
            surface_elevated: c(0x141414),
            code_bg: c(0x050505),
            inline_code_bg: c(0x161616),
            border: c(0x1c1c1c),
            border_focus: c(0x7a7a7a),
            input_border: c(0x262626),
            accent: c(0xd9d9d9),
            text: c(0xd9d9d9),
            text_strong: c(0xf5f5f5),
            text_secondary: c(0xa0a0a0),
            text_muted: c(0x6e6e6e),
            ok: c(0x8ccf9e),
            warning: c(0xe3b96f),
            error: c(0xf28b8b),
            field: c(0x000000),
            selection: rgba(0x3a83f75c).into(),
        }
    }

    /// Desktop's "Light Neutral" (palettes.rs `light_neutral`).
    pub fn light_neutral() -> Self {
        Self {
            background: c(0xf6f6f6),
            raised: c(0xf4f4f4),
            surface: c(0xf4f4f4),
            surface_elevated: c(0xe9eaea),
            code_bg: c(0xf7f7f7),
            inline_code_bg: c(0xeeeeee),
            border: c(0xe6e6e6),
            border_focus: c(0x858585),
            input_border: c(0xd9d9d9),
            accent: c(0x3b3d3f),
            text: c(0x3b3d3f),
            text_strong: c(0x1a1c1f),
            text_secondary: c(0x555555),
            text_muted: c(0x666666),
            ok: c(0x306844),
            warning: c(0x805518),
            error: c(0xb42332),
            field: c(0xffffff),
            selection: rgba(0x3a83f730).into(),
        }
    }

    pub fn get(cx: &App) -> Theme {
        *cx.global::<Theme>()
    }

    /// Pure black sets PROMPT_TINT_STRENGTH to 0: prompts are plain USER_BG.
    /// Light Neutral likewise uses plain USER_BG.
    pub fn prompt_background(_distance: usize) -> Hsla {
        if crate::system::is_light() { c(0xf4f4f4) } else { c(0x121212) }
    }
}

pub fn with_alpha(mut c: Hsla, a: f32) -> Hsla {
    c.a = a;
    c
}
