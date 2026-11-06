//! Compact voice status. No gesture ruler, moving puck, or transcript displacement.
//! Activity is symbolic, not a live transcript or measured audio waveform.
use crate::theme::{Theme, with_alpha};
use gpui::{AnyElement, div, prelude::*, px};

pub const HEIGHT: f32 = 44.0;

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

// Three staggered text-like strokes quietly indicate work without simulating speech.
fn stroke_alpha(index: usize, elapsed: f32, reduce_motion: bool) -> f32 {
    if reduce_motion || !elapsed.is_finite() {
        return 0.6;
    }
    0.3 + 0.5 * (elapsed.max(0.) * 3. - index as f32 * 0.8).sin().max(0.)
}

pub fn render(_pull: f32, phase: Phase, elapsed: f32, reduce_motion: bool, t: &Theme) -> AnyElement {
    let ink = if phase == Phase::Error { t.error } else { t.text_secondary };
    let animated = matches!(phase, Phase::Preparing | Phase::Recording | Phase::Locked | Phase::Transcribing);
    let mut activity = div().w(px(22.)).flex_none().flex().flex_col().gap(px(3.));
    for (i, width) in [18., 13., 16.].into_iter().enumerate() {
        activity = activity.child(
            div().w(px(width)).h(px(2.)).rounded_full()
                .bg(with_alpha(ink, stroke_alpha(i, elapsed, reduce_motion || !animated))),
        );
    }
    div()
        .id("voice-control")
        .debug_selector(|| "voice-control".into())
        .role(gpui::Role::Status)
        .aria_label(format!("{}. {}", phase.label(), phase.instruction()))
        .w_full().h(px(HEIGHT)).flex_none()
        .flex().items_center().justify_center().gap(px(8.))
        .bg(t.surface)
        .text_size(px(12.)).text_color(ink)
        .child(activity)
        .child(phase.label())
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
    fn animation_is_bounded_and_reduce_motion_is_static() {
        for i in 0..3 {
            for time in [-1., 0., 0.5, 100., f32::NAN, f32::INFINITY] {
                assert!((0.3..=0.8).contains(&stroke_alpha(i, time, false)));
                assert_eq!(stroke_alpha(i, time, true), 0.6);
            }
        }
        assert_ne!(stroke_alpha(0, 0., false), stroke_alpha(0, 0.5, false));
        assert_eq!(HEIGHT, 44.);
    }
}
