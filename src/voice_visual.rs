//! Bare microphone revealed beyond the transcript and composer by overscrolling.
//! Capture state remains available to accessibility without extra visual chrome.
use crate::theme::Theme;
use gpui::{AnyElement, div, prelude::*, px};

pub const HEIGHT: f32 = crate::voice_gesture::MAX_PULL;
pub const PROCESSING_REST: f32 = 44.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pulling,
    Preparing,
    Recording,
    Locked,
    Transcribing,
    Error,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pulling => "Pull to speak",
            Self::Preparing => "Preparing microphone",
            Self::Recording => "Listening",
            Self::Locked => "Listening · locked",
            Self::Transcribing => "Transcribing",
            Self::Error => "Microphone unavailable",
        }
    }
    fn instruction(self) -> &'static str {
        match self {
            Self::Pulling => "Pull to start recording",
            Self::Preparing => "Waiting for microphone, not recording",
            Self::Recording => "Move back down to finish, or release while deeply pulled to lock",
            Self::Locked => "Tap microphone to stop and send, or use cancel in the input line to discard",
            Self::Transcribing => "Turning your voice into text",
            Self::Error => "Check microphone access and try again",
        }
    }
}

pub fn reveal_height(pull: f32, _phase: Phase) -> f32 {
    // Presentation never adds a floor or clips lock travel. The recognizer owns
    // held travel and the spring, including the compact processing rest.
    if pull.is_finite() { pull.clamp(0., HEIGHT) } else { 0. }
}

fn mic_opacity(height: f32) -> f32 {
    let progress = (height / crate::voice_gesture::START).clamp(0., 1.);
    progress * progress * (3. - 2. * progress)
}

const HISTORY_LEN: usize = 24;
const SIDE_BARS: usize = HISTORY_LEN / 2;
const BAR_WIDTH: f32 = 2.;
const BAR_PITCH: f32 = 4.;
const SIDE_WIDTH: f32 = (SIDE_BARS - 1) as f32 * BAR_PITCH + BAR_WIDTH;
/// Width required by the waveform and microphone, excluding any touch padding.
pub const LIVE_WIDTH: f32 = SIDE_WIDTH * 2. + 20. + 12.;

fn waveform_amplitudes(levels: &[f32]) -> [f32; HISTORY_LEN] {
    let mut amplitudes = [0.; HISTORY_LEN];
    let recent = &levels[levels.len().saturating_sub(HISTORY_LEN)..];
    for (output, level) in amplitudes[HISTORY_LEN - recent.len()..].iter_mut().zip(recent) {
        *output = if level.is_finite() { level.clamp(0., 1.) } else { 0. };
    }
    amplitudes
}

fn bar_geometry(index: usize, amplitude: f32) -> (f32, f32, f32) {
    let amplitude = if amplitude.is_finite() { amplitude.clamp(0., 1.) } else { 0. };
    let height = 2. + amplitude * 18.;
    ((index % SIDE_BARS) as f32 * BAR_PITCH, (26. - height) / 2., height)
}

pub fn render(pull: f32, phase: Phase, elapsed: f32, reduce_motion: bool, t: &Theme) -> AnyElement {
    render_live(pull, phase, elapsed, reduce_motion, t, &[])
}

/// Render the newest 24 normalized amplitude samples, oldest to newest.
/// Empty history and zero samples are static silence marks, never synthetic motion.
pub fn render_live(pull: f32, phase: Phase, _elapsed: f32, reduce_motion: bool, t: &Theme, levels: &[f32]) -> AnyElement {
    let height = reveal_height(pull, phase);
    let ink = match phase {
        Phase::Error => t.error,
        Phase::Recording | Phase::Locked => t.ok,
        _ => t.accent,
    };
    // A drawn microphone rather than a font glyph keeps the icon consistent on iOS/Linux.
    let microphone = div()
        .debug_selector(|| "voice-overscroll-mic".into())
        .relative().w(px(20.)).h(px(26.)).flex_none()
        .child(div().absolute().left(px(6.)).top_0().w(px(8.)).h(px(15.))
            .rounded_full().bg(ink))
        .child(div().absolute().left(px(2.)).top(px(8.)).w(px(16.)).h(px(13.))
            .border_2().border_t_0().border_color(ink).rounded_b(px(8.)))
        .child(div().absolute().left(px(9.)).top(px(20.)).w(px(2.)).h(px(5.)).bg(ink))
        .child(div().absolute().left(px(5.)).bottom_0().w(px(10.)).h(px(2.)).rounded_full().bg(ink));
    let amplitudes = waveform_amplitudes(levels);
    let side = |start: usize| {
        div().relative().w(px(SIDE_WIDTH)).h(px(26.)).flex_none()
            .children((0..SIDE_BARS).map(|index| {
                let (left, top, height) = bar_geometry(index, amplitudes[start + index]);
                div().absolute().left(px(left)).top(px(top))
                    .w(px(BAR_WIDTH)).h(px(height)).rounded_full().bg(ink)
            }))
    };
    // Reduced motion only removes the decorative reveal easing. Live signal
    // samples still update directly, with no interpolation or elapsed-time animation.
    let opacity = if reduce_motion { if height > 0. { 1. } else { 0. } } else { mic_opacity(height) };
    let signal = div().flex().items_center().gap(px(6.)).w(px(LIVE_WIDTH)).h(px(26.)).flex_none()
        .debug_selector(|| "voice-live-waveform".into())
        .child(side(0)).child(microphone).child(side(SIDE_BARS));
    div()
        .id("voice-control")
        .debug_selector(|| "voice-control".into())
        .role(gpui::Role::Status)
        .aria_label(format!("{}. {}", phase.label(), phase.instruction()))
        .w_full().h(px(height)).flex_none().overflow_hidden()
        .flex().items_center().justify_center()
        .child(signal.opacity(opacity))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waveform_uses_only_recent_actual_samples_and_pads_silence() {
        assert_eq!(waveform_amplitudes(&[]), [0.; HISTORY_LEN]);
        let short = waveform_amplitudes(&[0.25, 0.75]);
        assert_eq!(&short[..22], &[0.; 22]);
        assert_eq!(&short[22..], &[0.25, 0.75]);
        let history: Vec<_> = (0..30).map(|n| n as f32 / 30.).collect();
        assert_eq!(waveform_amplitudes(&history).as_slice(), &history[6..]);
    }
    #[test]
    fn waveform_sanitizes_nonfinite_and_out_of_range_levels() {
        let levels = waveform_amplitudes(&[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1., 2., 0.5]);
        assert_eq!(&levels[18..], &[0., 0., 0., 0., 1., 0.5]);
    }
    #[test]
    fn waveform_geometry_stays_inside_existing_mic_height() {
        assert_eq!(LIVE_WIDTH, 124.);
        for index in 0..HISTORY_LEN {
            for amplitude in [0., 0.5, 1., -1., 2., f32::NAN, f32::INFINITY] {
                let (left, top, height) = bar_geometry(index, amplitude);
                assert!(left >= 0. && left + BAR_WIDTH <= SIDE_WIDTH);
                assert!((2. ..=20.).contains(&height));
                assert!(top >= 0. && top + height <= 26.);
                assert_eq!(top + height / 2., 13.);
            }
        }
        assert_eq!(bar_geometry(0, 0.), (0., 12., 2.));
        assert_eq!(bar_geometry(11, 1.), (44., 3., 20.));
    }
    #[test]
    fn preparing_never_claims_recording() {
        assert_eq!(Phase::Preparing.label(), "Preparing microphone");
        assert!(Phase::Preparing.instruction().contains("not recording"));
    }
    #[test]
    fn locked_explains_explicit_stop() {
        assert!(Phase::Locked.instruction().contains("Tap microphone to stop and send"));
        assert!(Phase::Recording.instruction().contains("Move back down to finish"));
        assert!(Phase::Recording.instruction().contains("release while deeply pulled to lock"));
    }
    #[test]
    fn mic_reveal_tracks_pull_and_is_bounded() {
        for phase in [Phase::Pulling, Phase::Recording] {
            assert_eq!(reveal_height(0., phase), 0.);
            assert_eq!(reveal_height(16., phase), 16.);
            assert_eq!(reveal_height(32., phase), 32.);
            assert_eq!(reveal_height(64., phase), 64.);
            assert_eq!(reveal_height(160., phase), HEIGHT);
            for pull in [-1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(reveal_height(pull, phase), 0.);
            }
        }
    }
    #[test]
    fn async_phase_changes_never_move_the_reveal() {
        for phase in [Phase::Pulling, Phase::Preparing, Phase::Recording, Phase::Locked, Phase::Transcribing, Phase::Error] {
            for pull in [0., 8., 16., 32., 44., 64., 96., 120., HEIGHT] {
                assert_eq!(reveal_height(pull, phase), pull);
            }
        }
    }
    #[test]
    fn opacity_eases_in_without_changing_icon_geometry() {
        assert_eq!(mic_opacity(0.), 0.);
        assert_eq!(mic_opacity(crate::voice_gesture::START / 2.), 0.5);
        assert_eq!(mic_opacity(crate::voice_gesture::START), 1.);
        let mut previous = 0.;
        for height in 0..=160 {
            let opacity = mic_opacity(height as f32);
            assert!((previous..=1.).contains(&opacity));
            previous = opacity;
        }
    }
}
