//! Design tokens mirrored from the Swift app's Theme.swift, itself mirrored
//! from Jcode Desktop's "Warm neutral" theme, so all three clients match.

use gpui::{App, Global, Hsla, Rgba, rgb};

#[derive(Clone, Copy)]
pub struct Theme {
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_elevated: Hsla,
    pub code_bg: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub text: Hsla,
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
    pub fn warm_neutral() -> Self {
        let mut selection: Hsla = c(0xB6A08A);
        selection.a = 0.3;
        Self {
            background: c(0x1C1A18),
            surface: c(0x25221F),
            surface_elevated: c(0x302B27),
            code_bg: c(0x1C1A18),
            border: c(0x403A34),
            accent: c(0xB6A08A),
            on_accent: c(0x1C1A18),
            text: c(0xE4DDD3),
            text_secondary: c(0xA79D91),
            text_muted: c(0x9B9084),
            ok: c(0xA6B68E),
            warning: c(0xD0B17D),
            error: c(0xE18C85),
            field: c(0x302B27),
            selection,
        }
    }

    pub fn get(cx: &App) -> Theme {
        *cx.global::<Theme>()
    }

    /// Desktop's prompt wash: newest prompt violet, older ones step toward red
    /// while the tint fades back into card paper.
    pub fn prompt_background(distance: usize) -> Hsla {
        const RAINBOW: [u32; 7] = [0xFF5050, 0xFFA050, 0xFFE650, 0x50DC64, 0x50C8DC, 0x648CFF, 0xB464FF];
        let tint = RAINBOW[RAINBOW.len() - 1 - distance.min(RAINBOW.len() - 1)];
        let amount = 0.05 * (-0.4 * distance as f32).exp();
        let ch = |v: u32, s: u32| ((v >> s) & 0xFF) as f32 / 255.0;
        let base = 0x302B27;
        let mix = |s| ch(base, s) * (1.0 - amount) + ch(tint, s) * amount;
        Rgba { r: mix(16), g: mix(8), b: mix(0), a: 1.0 }.into()
    }
}

pub fn with_alpha(mut c: Hsla, a: f32) -> Hsla {
    c.a = a;
    c
}
