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
    elapsed: f64,
    tick: u32,
    gesture: VoiceGesture,
    raw: f32,
    cycle: i32,
    phase: Phase,
}

impl VoiceDemo {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            elapsed: 0.,
            tick: 0,
            gesture: VoiceGesture::default(),
            raw: 0.,
            cycle: -1,
            phase: Phase::Pulling,
        }
    }

    /// Advance without wall-clock or native services. The script repeats every
    /// two nine-second cycles. Reconstruct only the current cycle after a gap:
    /// its reset makes all earlier history irrelevant, including spring state.
    /// Integer periodic ticks avoid both f32 increment stalls and huge loops.
    /// Returns the work count so the catch-up bound is directly testable.
    fn advance(&mut self, elapsed: f64) -> u32 {
        const HZ: u32 = 120;
        const CYCLE: u32 = 9 * HZ;
        const PERIOD: u32 = 2 * CYCLE;
        if !elapsed.is_finite() || elapsed < self.elapsed {
            return 0;
        }
        let target = ((elapsed % 18.) * HZ as f64).floor() as u32;
        if elapsed - self.elapsed >= 9. || self.cycle < 0 {
            self.tick = target / CYCLE * CYCLE;
            self.gesture.reset();
            self.raw = 0.;
            self.cycle = -1;
            self.step(self.tick as f32 / HZ as f32, 0.);
        }
        let steps = (target + PERIOD - self.tick) % PERIOD;
        for _ in 0..steps {
            self.tick = (self.tick + 1) % PERIOD;
            self.step(self.tick as f32 / HZ as f32, 1. / HZ as f32);
        }
        self.elapsed = elapsed;
        steps
    }

    /// Reduced motion keeps the scripted state accurate on explicit redraws,
    /// but does not pump animation frames or animate the recognizer's spring.
    fn preview_frame(&mut self, elapsed: f64, reduce_motion: bool) -> bool {
        self.advance(elapsed);
        if reduce_motion {
            self.gesture.snap_to_target();
        }
        !reduce_motion
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
            // The mic itself stops capture in the real UI without resetting travel.
            self.gesture.dismiss();
        } else {
            self.gesture.release();
        }
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
        let rest = if self.gesture.held() { 0. }
            else if self.phase == Phase::Locked { crate::voice_gesture::LOCK_REST }
            else if self.phase == Phase::Transcribing { voice_visual::PROCESSING_REST }
            else { 0. };
        self.gesture.rest_at(rest);
        self.gesture.tick(dt);
    }
}

impl Render for VoiceDemo {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed = self.started.elapsed().as_secs_f64();
        let reduce_motion = crate::system::reduce_motion();
        if self.preview_frame(elapsed, reduce_motion) {
            window.request_animation_frame();
        }
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
                        "Release while deeply pulled to lock. Move back down before releasing to finish instead."
                    } else { "Pull past START to speak. Reverse to STOP or release to send." }))
                .child(div().mt(px(16.)).text_size(px(10.)).text_color(t.text_muted)
                    .child("Rhythmic activity preview, not measured audio.\nNo microphone access. No network requests."))
                .child(div().mt(px(20.)).text_size(px(11.)).text_color(t.ok)
                    .child(format!("SIMULATED / {}", self.phase.label()))))
            .child(voice_visual::render(self.gesture.pull, self.phase, elapsed as f32, reduce_motion, &t))
            .child(div().px(px(22.)).pt(px(14.)).pb(px(34.)).bg(t.raised)
                .child(div().p(px(13.)).rounded_lg().border_1().border_color(t.input_border)
                    .text_size(px(11.)).text_color(t.text_muted).child("Preview only · composer controls live in chat")))
    }
}

/// Open a phone-sized native window without initializing the real app services.
pub fn run() {
    gpui_platform::application().run(|cx: &mut App| {
        crate::load_fonts(cx);
        crate::system::refresh(cx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_demo_long_uptime_clock_progresses() {
        let mut demo = VoiceDemo::new();
        assert!(demo.advance(262_144.) <= 1080);
        let tick = demo.tick;
        assert!((1..=2).contains(&demo.advance(262_144. + 1. / 60.)));
        assert_ne!(demo.tick, tick);
    }

    #[test]
    fn voice_demo_reduce_motion_stops_frames_without_changing_script() {
        for elapsed in [1.7, 3., 5.6, 12.45, 15.5, 262_144.] {
            let mut normal = VoiceDemo::new();
            let mut reduced = VoiceDemo::new();
            assert!(normal.preview_frame(elapsed, false));
            assert!(!reduced.preview_frame(elapsed, true));
            assert_eq!(normal.phase, reduced.phase);
            assert_eq!(normal.gesture.state, reduced.gesture.state);
            assert_eq!(normal.cycle, reduced.cycle);
            assert!(!reduced.gesture.tick(1. / 120.));
            assert!(!reduced.preview_frame(elapsed, true));
            assert!(reduced.preview_frame(elapsed + 0.5, false));
            let mut fresh = VoiceDemo::new();
            fresh.advance(elapsed + 0.5);
            assert_eq!(reduced.phase, fresh.phase);
            assert_eq!(reduced.gesture.state, fresh.gesture.state);
        }
    }

    fn same(actual: &VoiceDemo, expected: &VoiceDemo) {
        assert_eq!(actual.cycle, expected.cycle);
        assert_eq!(actual.phase, expected.phase);
        assert_eq!(actual.gesture.state, expected.gesture.state);
        assert!((actual.gesture.pull - expected.gesture.pull).abs() < 0.001);
        assert!((actual.raw - expected.raw).abs() < 0.001);
    }

    #[test]
    fn voice_demo_slow_frames_preserve_fixed_step_semantics() {
        let mut reference = VoiceDemo::new();
        let mut slow = VoiceDemo::new();
        for tick in 1..=4320 {
            reference.advance(tick as f64 / 120.);
            if tick % 83 == 0 || tick == 4320 {
                assert!(slow.advance(tick as f64 / 120.) <= 1080);
                same(&slow, &reference);
            }
        }
    }

    #[test]
    fn voice_demo_suspend_reconstructs_current_cycle_not_stale_lock() {
        for elapsed in [18., 19.7, 23.6, 27., 30.45, 33.2, 35.5, 262_144.] {
            let mut resumed = VoiceDemo::new();
            resumed.advance(12.5);
            assert_eq!(resumed.gesture.state, State::Locked);
            assert!(resumed.advance(elapsed) <= 1080);
            let mut fresh = VoiceDemo::new();
            fresh.advance(elapsed % 18.);
            same(&resumed, &fresh);
        }
    }

    #[test]
    fn voice_demo_huge_nonfinite_and_backward_time_are_safe() {
        let mut demo = VoiceDemo::new();
        demo.advance(3.);
        for elapsed in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1., 2.] {
            let tick = demo.tick;
            assert_eq!(demo.advance(elapsed), 0);
            assert_eq!(demo.tick, tick);
            assert_eq!(demo.gesture.state, State::Recording);
        }
        for elapsed in [1e12, 1e30, f64::MAX] {
            assert!(demo.advance(elapsed) <= 1080);
            assert!(demo.gesture.pull.is_finite());
            let mut fresh = VoiceDemo::new();
            fresh.advance(elapsed % 18.);
            same(&demo, &fresh);
            assert_eq!(demo.advance(elapsed), 0);
        }
    }

    #[test]
    fn voice_demo_cycle_phases_and_thresholds() {
        let mut demo = VoiceDemo::new();
        for (elapsed, state, phase) in [
            (0.7, State::Idle, Phase::Pulling),
            (1.62, State::Pulling, Phase::Pulling),
            (1.63, State::Recording, Phase::Preparing),
            (2.06, State::Recording, Phase::Recording),
            (5.48, State::Recording, Phase::Recording),
            (5.50, State::Finished, Phase::Transcribing),
            (5.8, State::Idle, Phase::Transcribing),
            (8.1, State::Idle, Phase::Pulling),
            (9., State::Idle, Phase::Pulling),
            (12.38, State::Recording, Phase::Recording),
            (12.40, State::Recording, Phase::Recording),
            (12.50, State::Locked, Phase::Locked),
            (15.29, State::Locked, Phase::Locked),
            (15.31, State::Idle, Phase::Transcribing),
            (17.1, State::Idle, Phase::Pulling),
            (18., State::Idle, Phase::Pulling),
        ] {
            demo.advance(elapsed);
            assert_eq!(demo.gesture.state, state, "elapsed={elapsed}");
            assert_eq!(demo.phase, phase, "elapsed={elapsed}");
        }
    }

    #[test]
    fn voice_demo_exact_phase_boundaries() {
        let mut demo = VoiceDemo::new();
        for (elapsed, phase) in [
            (2.049, Phase::Preparing),
            (2.05, Phase::Recording),
            (5.549, Phase::Transcribing),
            (5.55, Phase::Transcribing),
            (8.1, Phase::Pulling),
            (15.299, Phase::Locked),
            (15.3, Phase::Transcribing),
            (17.1, Phase::Pulling),
        ] {
            demo.advance(elapsed as f64);
            demo.step(elapsed, 1. / 120.);
            assert_eq!(demo.phase, phase, "elapsed={elapsed}");
        }
    }

    #[gpui::test]
    fn voice_demo_narrow_preview_geometry(cx: &mut gpui::TestAppContext) {
        // Construct only the autonomous preview. No app init, capture, networking,
        // platform application, or microphone fixture is involved.
        let (view, cx) = cx.add_window_view(|_, _| VoiceDemo::new());
        for width in [280., 320., 390.] {
            cx.simulate_resize(size(px(width), px(844.)));
            for elapsed in [3., 12.5, 15.5] {
                view.update(cx, |demo, _| {
                    *demo = VoiceDemo::new();
                    demo.advance(elapsed);
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let bounds = cx.debug_bounds("voice-control").expect("voice status");
                assert_eq!(bounds.size.width, px(width));
                let expected = view.read_with(cx, |demo, _| {
                    voice_visual::reveal_height(demo.gesture.pull, demo.phase)
                });
                assert!((f32::from(bounds.size.height) - expected).abs() <= 1.);
                assert!(bounds.origin.x >= px(0.));
                assert!(bounds.origin.y >= px(0.));
                assert!(bounds.bottom() <= px(844.));
            }
        }
    }
}
