//! Safe, autonomous interaction preview. No capture or networking is initialized.
use crate::{
    theme::{self, Theme},
    voice_gesture::{State, VoiceGesture},
    voice_visual::{self, Phase},
};
use gpui::{
    App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use std::time::Instant;

struct VoiceDemo {
    started: Instant,
    simulated: f32,
    gesture: VoiceGesture,
    raw: f32,
    cycle: i32,
    phase: Phase,
}

impl VoiceDemo {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            simulated: 0.,
            gesture: VoiceGesture::default(),
            raw: 0.,
            cycle: -1,
            phase: Phase::Pulling,
        }
    }

    fn step(&mut self, elapsed: f32, dt: f32) {
        let cycle = (elapsed / 9.) as i32;
        let local = elapsed % 9.;
        let locking = cycle % 2 == 1;
        if cycle != self.cycle {
            self.gesture.reset();
            self.raw = 0.;
            self.cycle = cycle;
        }
        // Feed actual finger deltas into the shared recognizer. Never assign its
        // state or pull, so the preview exercises threshold and snap behavior.
        let visible = if local < 0.7 {
            0.
        } else if local < 2.2 {
            ((local - 0.7) / 1.5 * 52.).min(52.)
        } else if locking && local < 3.5 {
            52. + (local - 2.2) / 1.3 * 48.
        } else if !locking && local < 4.8 {
            52.
        } else if !locking && local < 5.8 {
            52. * (5.8 - local)
        } else {
            0.
        };
        if local < if locking { 3.5 } else { 5.8 } {
            let raw = visible / (1. - visible / 160.);
            self.gesture.feed(self.raw - raw, false);
            self.raw = raw;
        } else if locking && local < 6.3 {
            self.gesture.release();
        } else if locking {
            // This is the existing composer's explicit stop action in the real UI.
            self.gesture.reset();
        } else {
            self.gesture.release();
        }
        self.gesture.tick(dt);
        let sending = local >= if locking { 6.3 } else { 5.55 };
        self.phase = if sending && local < 8.1 {
            Phase::Transcribing
        } else if sending {
            Phase::Pulling
        } else {
            match self.gesture.state {
                State::Locked => Phase::Locked,
                State::Recording if local < 2.05 => Phase::Preparing,
                State::Recording => Phase::Recording,
                State::Finished => Phase::Transcribing,
                State::Idle | State::Pulling => Phase::Pulling,
            }
        };
    }
}

impl Render for VoiceDemo {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed = self.started.elapsed().as_secs_f32();
        // Fixed substeps preserve every crossing even during a slow video frame.
        while self.simulated + 1. / 120. <= elapsed {
            self.simulated += 1. / 120.;
            self.step(self.simulated, 1. / 120.);
        }
        window.request_animation_frame();
        let t = Theme::pure_black();
        let locked_cycle = self.cycle % 2 == 1;
        div().size_full().flex().flex_col().bg(t.background)
            .font_family(theme::FONT).text_color(t.text)
            .child(div().pt(px(52.)).px(px(24.)).pb(px(22.)).border_b_1().border_color(t.border)
                .child(div().text_size(px(19.)).child("jcode / voice"))
                .child(div().mt(px(9.)).text_size(px(10.)).text_color(t.ok)
                    .child("Interaction preview • simulated audio")))
            .child(div().flex_1().min_h_0().flex().flex_col().justify_end().px(px(24.)).pb(px(26.))
                .child(div().text_size(px(10.)).text_color(t.text_muted)
                    .child(if locked_cycle { "02 / HANDS-FREE CAPTURE" } else { "01 / SHORT PULL TO TALK" }))
                .child(div().mt(px(16.)).text_size(px(22.)).text_color(t.text_strong)
                    .child(if locked_cycle { "Pull farther.\nKeep your flow." } else { "A little pull.\nA whole thought." }))
                .child(div().mt(px(20.)).p(px(14.)).rounded_lg().bg(t.surface)
                    .text_size(px(12.)).child(if locked_cycle {
                        "Pull to LOCK, then let go. Recording stays active until Stop sends."
                    } else { "Pull past START to speak. Reverse to STOP or release to send." }))
                .child(div().mt(px(16.)).text_size(px(10.)).text_color(t.text_muted)
                    .child("Rhythmic activity preview, not measured audio.\nNo microphone access. No network requests."))
                .child(div().mt(px(20.)).text_size(px(11.)).text_color(t.ok)
                    .child(format!("SIMULATED / {}", self.phase.label()))))
            .child(voice_visual::render(self.gesture.pull, self.phase, elapsed, false, &t))
            .child(div().px(px(22.)).pt(px(14.)).pb(px(34.)).bg(t.raised)
                .child(div().p(px(13.)).rounded_lg().border_1().border_color(t.input_border)
                    .text_size(px(11.)).text_color(t.text_muted).child("Preview only · composer controls live in chat")))
    }
}

/// Open a phone-sized native window without initializing the real app services.
pub fn run() {
    gpui_platform::application().run(|cx: &mut App| {
        crate::load_fonts(cx);
        cx.set_global(Theme::pure_black());
        let bounds = Bounds::centered(None, size(px(390.), px(844.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("jcode-voice-preview".into()),
                ..Default::default()
            },
            |_, cx| cx.new(|_| VoiceDemo::new()),
        )
        .expect("open voice preview");
        cx.activate(true);
    });
}
