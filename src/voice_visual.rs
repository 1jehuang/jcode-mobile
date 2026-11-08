//! Bare microphone revealed beyond the transcript and composer by overscrolling.
//! Capture state remains available to accessibility without extra visual chrome.
use crate::theme::Theme;
use gpui::{AnyElement, div, prelude::*, px};

pub const HEIGHT: f32 = 96.0;

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
            Self::Recording => "Release to send, or pull farther to lock",
            Self::Locked => "Use Stop to send, or Cancel to discard",
            Self::Transcribing => "Turning your voice into text",
            Self::Error => "Check microphone access and try again",
        }
    }
}

pub fn reveal_height(pull: f32, phase: Phase) -> f32 {
    let pull = if pull.is_finite() { pull.max(0.) } else { 0. };
    // Keep processing visible after the released gesture has sprung back.
    let rest = if matches!(phase, Phase::Preparing | Phase::Transcribing | Phase::Error) {
        32.
    } else if phase == Phase::Locked {
        crate::voice_gesture::LOCK
    } else {
        0.
    };
    pull.max(rest).min(HEIGHT)
}

pub fn render(pull: f32, phase: Phase, _elapsed: f32, _reduce_motion: bool, t: &Theme) -> AnyElement {
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
    div()
        .id("voice-control")
        .debug_selector(|| "voice-control".into())
        .role(gpui::Role::Status)
        .aria_label(format!("{}. {}", phase.label(), phase.instruction()))
        .w_full().h(px(height)).flex_none().overflow_hidden()
        .flex().items_center().justify_center()
        .child(microphone.opacity((height / crate::voice_gesture::START).clamp(0., 1.)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparing_never_claims_recording() {
        assert_eq!(Phase::Preparing.label(), "Preparing microphone");
        assert!(Phase::Preparing.instruction().contains("not recording"));
    }
    #[test]
    fn locked_explains_explicit_stop() {
        assert!(Phase::Locked.instruction().contains("Stop to send"));
        assert!(Phase::Recording.instruction().contains("Release to send"));
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
    fn pending_work_remains_visible_after_release() {
        for phase in [Phase::Preparing, Phase::Transcribing, Phase::Error] {
            assert_eq!(reveal_height(0., phase), 32.);
            assert_eq!(reveal_height(64., phase), 64.);
        }
        assert_eq!(reveal_height(0., Phase::Locked), crate::voice_gesture::LOCK);
    }
}
