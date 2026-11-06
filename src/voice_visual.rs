//! Stateless voice affordance. `pull` is already rubber-banded display pixels.
//! Animation is driven exclusively by the owner's elapsed clock.
use crate::theme::{Theme, with_alpha};
use crate::voice_gesture::{LOCK, START, STOP};
use gpui::{AnyElement, div, prelude::*, px};

pub const HEIGHT: f32 = 156.0;

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
            Self::Locked => "Recording locked",
            Self::Transcribing => "Transcribing",
            Self::Error => "Microphone unavailable",
        }
    }
    fn instruction(self) -> &'static str {
        match self {
            Self::Pulling => "Short pull starts capture",
            Self::Preparing => "Waiting for microphone, not recording",
            Self::Recording => "Return to STOP or release to send",
            Self::Locked => "Hands free · use Stop to send",
            Self::Transcribing => "Turning your voice into text",
            Self::Error => "Check microphone access and try again",
        }
    }
}

pub fn render(pull: f32, phase: Phase, elapsed: f32, reduce_motion: bool, t: &Theme) -> AnyElement {
    let pull = if pull.is_finite() {
        pull.clamp(0., 108.)
    } else {
        0.
    };
    let time = if reduce_motion || !elapsed.is_finite() {
        0.
    } else {
        elapsed.max(0.)
    };
    let live = matches!(phase, Phase::Recording | Phase::Locked);
    let ink = match phase {
        Phase::Error => t.error,
        Phase::Preparing => t.warning,
        Phase::Recording | Phase::Locked => t.ok,
        _ => t.accent,
    };
    // Puck center and tick marks use precisely the same coordinate space.
    let origin = 132.;
    let mut panel = div()
        .id("voice-control")
        .debug_selector(|| "voice-control".into())
        .role(gpui::Role::Status)
        .aria_label(format!("{}. {}", phase.label(), phase.instruction()))
        .relative()
        .w_full()
        .h(px(HEIGHT))
        .flex_none()
        .overflow_hidden()
        .bg(t.raised)
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .absolute()
                .left(px(38.))
                .top(px(origin - 108.))
                .w(px(4.))
                .h(px(108.))
                .rounded_full()
                .bg(t.border),
        )
        .child(
            div()
                .absolute()
                .left(px(38.))
                .top(px(origin - pull))
                .w(px(4.))
                .h(px(pull))
                .rounded_full()
                .bg(with_alpha(ink, 0.55)),
        );
    for (travel, name) in [(STOP, "STOP"), (START, "START"), (LOCK, "LOCK")] {
        let active = if travel == LOCK {
            phase == Phase::Locked
        } else {
            pull >= travel
        };
        panel = panel.child(
            div()
                .absolute()
                .left(px(53.))
                .top(px(origin - travel - 6.))
                .h(px(12.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(9.))
                .text_color(if active { ink } else { t.text_muted })
                .child(
                    div()
                        .w(px(8.))
                        .h(px(1.))
                        .bg(if active { ink } else { t.border_focus }),
                )
                .child(format!("{name} {travel:.0}")),
        );
    }
    // A native vector microphone, assembled from rounded GPUI primitives.
    let mic = div()
        .relative()
        .size(px(18.))
        .child(
            div()
                .absolute()
                .left(px(6.))
                .top(px(0.))
                .w(px(6.))
                .h(px(11.))
                .rounded_full()
                .border_1()
                .border_color(ink),
        )
        .child(
            div()
                .absolute()
                .left(px(3.))
                .top(px(6.))
                .w(px(12.))
                .h(px(9.))
                .rounded_b_full()
                .border_b_1()
                .border_l_1()
                .border_r_1()
                .border_color(ink),
        )
        .child(
            div()
                .absolute()
                .left(px(8.))
                .top(px(14.))
                .w(px(2.))
                .h(px(4.))
                .bg(ink),
        );
    panel = panel.child(
        div()
            .absolute()
            .left(px(23.))
            .top(px(origin - pull - 17.))
            .size(px(34.))
            .rounded_full()
            .bg(t.surface)
            .border_1()
            .border_color(ink)
            .flex()
            .items_center()
            .justify_center()
            .child(mic),
    );
    let mut activity = div().h(px(27.)).flex().items_center().gap(px(3.));
    if live {
        for i in 0..19 {
            let wave =
                ((time * 5.2 + i as f32 * 0.79).sin() * (time * 2.1 - i as f32 * 0.37).cos()).abs();
            let envelope = 1. - ((i as f32 - 9.) / 12.).abs();
            let height = if reduce_motion {
                5. + envelope * 11.
            } else {
                4. + wave * envelope * 22.
            };
            activity = activity.child(
                div()
                    .w(px(3.))
                    .h(px(height))
                    .rounded_full()
                    .bg(with_alpha(ink, 0.45 + envelope * 0.5)),
            );
        }
    } else if matches!(phase, Phase::Preparing | Phase::Transcribing) {
        for i in 0..3 {
            let alpha = if reduce_motion {
                0.65
            } else {
                0.3 + 0.7 * (time * 3. - i as f32 * 0.8).sin().max(0.)
            };
            activity = activity.child(div().size(px(5.)).rounded_full().bg(with_alpha(ink, alpha)));
        }
    } else {
        activity = activity.child(div().w(px(72.)).h(px(2.)).rounded_full().bg(t.border_focus));
    }
    panel
        .child(
            div()
                .absolute()
                .left(px(143.))
                .right(px(12.))
                .top(px(23.))
                .flex()
                .flex_col()
                .gap(px(7.))
                .text_color(t.text)
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(ink)
                        .child(phase.label()),
                )
                .child(activity)
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(t.text_secondary)
                        .child(phase.instruction()),
                )
                .child(div().text_size(px(9.)).text_color(t.text_muted).child(
                    if phase == Phase::Locked {
                        "● LOCKED · stop sends"
                    } else {
                        "Pull farther to LOCK · 96 px"
                    },
                )),
        )
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
        assert!(Phase::Recording.instruction().contains("release to send"));
    }
}
