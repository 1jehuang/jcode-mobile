//! The jcode hero donut, ported 1:1 from jcode.sh (public/donut.js): the
//! website's own `site-visual` torus field, screened through the 45-degree
//! halftone lattice with the same gamma, radius and motion.

use std::time::Instant;

use crate::donut_field::{DonutOptions, render};
use gpui::{
    App, Bounds, Hsla, MouseButton, Pixels, Point, RenderOnce, Window, canvas, div, point,
    prelude::*, px, size,
};

/// Luminance field resolution (site: 96 to 152 adaptive).
const GRID: usize = 144;
/// Halftone density tuned for the 140-point onboarding hero. Wider spacing
/// keeps the dots separate instead of forming a shimmering checkerboard.
const CELLS: f32 = 32.0;

fn sample(lum: &[f32], gx: f32, gy: f32) -> f32 {
    let g = GRID as f32;
    if gx < 0.0 || gy < 0.0 || gx > g - 2.0 || gy > g - 2.0 {
        return 0.0;
    }
    let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
    let at = |x: usize, y: usize| lum[x + y * GRID];
    at(x0, y0) * (1.0 - fx) * (1.0 - fy)
        + at(x0 + 1, y0) * fx * (1.0 - fy)
        + at(x0, y0 + 1) * (1.0 - fx) * fy
        + at(x0 + 1, y0 + 1) * fx * fy
}

#[derive(Default)]
struct Rotation {
    last: Option<Point<Pixels>>,
    tilt: f64,
    spin: f64,
}

impl Rotation {
    fn touch_drag(&mut self, event: &gpui::TouchDragEvent, side: f32) {
        if event.phase == gpui::TouchPhase::Started {
            self.last = Some(event.start_position);
        }
        if event.phase != gpui::TouchPhase::Cancelled {
            self.drag_to(event.position, side);
        }
        if matches!(
            event.phase,
            gpui::TouchPhase::Ended | gpui::TouchPhase::Cancelled
        ) {
            self.last = None;
        }
    }

    fn drag_to(&mut self, position: Point<Pixels>, side: f32) {
        if let Some(last) = self.last {
            let scale = std::f64::consts::TAU / side.max(1.) as f64;
            self.spin += f32::from(position.x - last.x) as f64 * scale;
            self.tilt += f32::from(position.y - last.y) as f64 * scale;
            self.last = Some(position);
        }
    }
}

#[derive(IntoElement)]
struct InteractiveDonut {
    side: f32,
    ink: Hsla,
    started: Instant,
}

impl RenderOnce for InteractiveDonut {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { side, ink, started } = self;
        let rotation =
            window.use_keyed_state("hero-donut-rotation", cx, |_, _| Rotation::default());
        interactive_donut(side, ink, started, rotation, cx)
    }
}

fn interactive_donut(
    side: f32,
    ink: Hsla,
    started: Instant,
    rotation: gpui::Entity<Rotation>,
    cx: &App,
) -> impl IntoElement {
    let tilt = rotation.read(cx).tilt;
    let spin = rotation.read(cx).spin;
    let down = rotation.clone();
    let movement = rotation.clone();
    let up = rotation.clone();
    let touch = rotation.clone();
    div()
        .id("interactive-hero-donut")
        .debug_selector(|| "interactive-hero-donut".into())
        .relative()
        .size(px(side))
        .flex_none()
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            // The hero is outside the form even when it captures a drag.
            window.blur(cx);
            window.dismiss_virtual_keyboard();
            down.update(cx, |state, cx| {
                state.last = Some(event.position);
                cx.notify();
            });
            cx.stop_propagation();
        })
        .on_mouse_move(move |event, _, cx| {
            movement.update(cx, |state, cx| {
                if !event.dragging() {
                    state.last = None;
                } else if state.last.is_some() {
                    state.drag_to(event.position, side);
                    cx.notify();
                }
            });
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            up.update(cx, |state, _| state.last = None);
        })
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
            rotation.update(cx, |state, _| state.last = None);
        })
        .child(donut_canvas(side, ink, started, tilt, spin))
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let rotation = touch.clone();
                    window.on_mouse_event(
                        move |event: &gpui::TouchDragEvent, phase, window, cx| {
                            if phase != gpui::DispatchPhase::Bubble
                                || !bounds.contains(&event.start_position)
                            {
                                return;
                            }
                            rotation.update(cx, |state, cx| {
                                if event.phase == gpui::TouchPhase::Started {
                                    window.blur(cx);
                                    window.dismiss_virtual_keyboard();
                                }
                                state.touch_drag(event, side);
                                cx.notify();
                            });
                            window.prevent_default();
                            cx.stop_propagation();
                        },
                    );
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size(px(side)),
        )
}

pub fn spinning_donut(side: f32, ink: Hsla, started: Instant) -> impl IntoElement {
    InteractiveDonut { side, ink, started }
}

fn donut_canvas(side: f32, ink: Hsla, started: Instant, tilt: f64, spin: f64) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
            let t = started.elapsed().as_secs_f64();
            // Same orientation as makeDonut().render() on the site.
            let mut lum = vec![0.0f32; GRID * GRID];
            render(
                DonutOptions {
                    columns: GRID,
                    rows: GRID,
                    tilt: 1.0 + (t * 0.4).sin() * 0.25 + tilt,
                    spin: t * 0.7 + spin,
                    steps: 48,
                    refine: true,
                    sharp: false,
                },
                &mut lum,
            );
            let w = f32::from(bounds.size.width);
            let s = w / CELLS;
            let half = w / 2.0;
            let (sin_a, cos_a) = std::f32::consts::FRAC_PI_4.sin_cos();
            let ext = (CELLS * std::f32::consts::FRAC_1_SQRT_2).ceil() as i32 + 1;
            for j in -ext..=ext {
                for i in -ext..=ext {
                    let (i, j) = (i as f32, j as f32);
                    let px_ = half + (i * cos_a - j * sin_a) * s;
                    let py_ = half + (i * sin_a + j * cos_a) * s;
                    if px_ < -s || px_ > w + s || py_ < -s || py_ > w + s {
                        continue;
                    }
                    let v = sample(&lum, px_ / w * GRID as f32, py_ / w * GRID as f32);
                    if v <= 0.04 {
                        continue;
                    }
                    let r = v.powf(0.85) * s * 0.48;
                    window.paint_quad(
                        gpui::fill(
                            Bounds::new(
                                point(bounds.origin.x + px(px_ - r), bounds.origin.y + px(py_ - r)),
                                size(px(2.0 * r), px(2.0 * r)),
                            ),
                            ink,
                        )
                        .corner_radii(px(r)),
                    );
                }
            }
            window.request_animation_frame();
        },
    )
    .size(px(side))
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DonutFixture {
        rotation: gpui::Entity<Rotation>,
    }

    impl gpui::Render for DonutFixture {
        fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
            interactive_donut(
                140.,
                gpui::white(),
                Instant::now(),
                self.rotation.clone(),
                cx,
            )
        }
    }

    #[gpui::test]
    fn raw_touch_rotates_the_rendered_donut(cx: &mut gpui::TestAppContext) {
        use gpui::{PlatformInput, TouchEvent, TouchId, TouchPhase};
        let (view, cx) = cx.add_window_view(|_, cx| DonutFixture {
            rotation: cx.new(|_| Rotation::default()),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bounds = cx
            .debug_bounds("interactive-hero-donut")
            .expect("donut hit target");
        let start = bounds.center();
        let end = start + point(px(35.), px(20.));
        for (phase, position) in [
            (TouchPhase::Started, start),
            (TouchPhase::Moved, end),
            (TouchPhase::Ended, end),
        ] {
            cx.update(|window, cx| {
                window.dispatch_event(
                    PlatformInput::Touch(TouchEvent {
                        id: TouchId(1),
                        phase,
                        position,
                        predicted_position: None,
                        force: None,
                    }),
                    cx,
                );
                window.draw(cx).clear(cx);
            });
        }
        view.read_with(cx, |view, cx| {
            let rotation = view.rotation.read(cx);
            assert!(rotation.spin.abs() > 0.5, "raw touch must rotate the hero");
            assert!(
                rotation.tilt.abs() > 0.3,
                "vertical drag must tilt the hero"
            );
            assert!(rotation.last.is_none(), "release must end the drag");
        });
    }

    #[test]
    fn touch_drag_rotates_from_contact_origin_and_cancels_without_jumping() {
        use gpui::{TouchDragEvent, TouchPhase};
        let mut rotation = Rotation::default();
        let mut event = TouchDragEvent {
            phase: TouchPhase::Started,
            start_position: point(px(10.), px(20.)),
            position: point(px(45.), px(55.)),
        };
        rotation.touch_drag(&event, 140.);
        assert!((rotation.spin - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        assert!((rotation.tilt - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        event.phase = TouchPhase::Moved;
        event.position = point(px(80.), px(90.));
        rotation.touch_drag(&event, 140.);
        assert!((rotation.spin - std::f64::consts::PI).abs() < 1e-10);
        let orientation = (rotation.tilt, rotation.spin);
        event.phase = TouchPhase::Cancelled;
        event.position = point(px(400.), px(400.));
        rotation.touch_drag(&event, 140.);
        assert_eq!((rotation.tilt, rotation.spin), orientation);
        assert!(rotation.last.is_none());
        event.phase = TouchPhase::Moved;
        rotation.touch_drag(&event, 140.);
        assert_eq!((rotation.tilt, rotation.spin), orientation);
    }

    #[test]
    fn touch_release_applies_final_position_and_ends_drag() {
        use gpui::{TouchDragEvent, TouchPhase};
        let mut rotation = Rotation::default();
        let mut event = TouchDragEvent {
            phase: TouchPhase::Started,
            start_position: point(px(0.), px(0.)),
            position: point(px(10.), px(10.)),
        };
        rotation.touch_drag(&event, 140.);
        event.phase = TouchPhase::Ended;
        event.position = point(px(70.), px(35.));
        rotation.touch_drag(&event, 140.);
        assert!((rotation.spin - std::f64::consts::PI).abs() < 1e-10);
        assert!((rotation.tilt - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        assert!(rotation.last.is_none());
    }

    #[test]
    fn movement_without_press_does_not_rotate() {
        let mut rotation = Rotation::default();
        rotation.drag_to(point(px(70.), px(35.)), 140.);
        assert_eq!((rotation.tilt, rotation.spin), (0., 0.));
        assert!(rotation.last.is_none());
    }

    #[test]
    fn drag_rotates_both_axes_and_tracks_incremental_motion() {
        let origin = point(px(10.), px(20.));
        let mut rotation = Rotation {
            last: Some(origin),
            ..Default::default()
        };
        rotation.drag_to(point(px(80.), px(55.)), 140.);
        assert!((rotation.spin - std::f64::consts::PI).abs() < 1e-10);
        assert!((rotation.tilt - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        rotation.drag_to(origin, 140.);
        assert!(rotation.spin.abs() < 1e-10);
        assert!(rotation.tilt.abs() < 1e-10);
    }

    #[test]
    fn release_preserves_orientation_and_ignores_further_movement() {
        let mut rotation = Rotation {
            last: Some(point(px(0.), px(0.))),
            ..Default::default()
        };
        rotation.drag_to(point(px(20.), px(30.)), 140.);
        let orientation = (rotation.tilt, rotation.spin);
        rotation.last = None;
        rotation.drag_to(point(px(100.), px(100.)), 140.);
        assert_eq!((rotation.tilt, rotation.spin), orientation);
    }
}
