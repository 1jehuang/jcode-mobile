//! The jcode hero donut, ported 1:1 from jcode.sh (public/donut.js): the
//! website's own `site-visual` torus field, screened through the 45-degree
//! halftone lattice with the same gamma, radius and motion.

use std::{cell::RefCell, rc::Rc, time::Instant};

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

/// Keep renderer inputs bounded even after long uptimes or hostile drag input.
fn angle(value: f64) -> f64 {
    if value.is_finite() {
        value % std::f64::consts::TAU
    } else {
        0.0
    }
}

fn orientation(elapsed: f64, reduce_motion: bool, tilt: f64, spin: f64) -> (f64, f64, bool) {
    let elapsed = if elapsed.is_finite() {
        elapsed.max(0.)
    } else {
        0.
    };
    let (wobble, turn) = if reduce_motion {
        (0., 0.)
    } else {
        (angle(elapsed * 0.4).sin() * 0.25, angle(elapsed * 0.7))
    };
    (
        angle(1.0 + wobble + angle(tilt)),
        angle(turn + angle(spin)),
        !reduce_motion,
    )
}

#[derive(Clone, Copy)]
struct Sample {
    offset: usize,
    fx: f32,
    fy: f32,
}

impl Sample {
    fn new(gx: f32, gy: f32) -> Option<Self> {
        let g = GRID as f32;
        if gx < 0.0 || gy < 0.0 || gx > g - 2.0 || gy > g - 2.0 {
            return None;
        }
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
        Some(Self {
            offset: x0 + y0 * GRID,
            fx,
            fy,
        })
    }

    fn read(self, lum: &[f32]) -> f32 {
        let Self { offset, fx, fy } = self;
        // Preserve the original multiply/add order, not precombined weights.
        lum[offset] * (1.0 - fx) * (1.0 - fy)
            + lum[offset + 1] * fx * (1.0 - fy)
            + lum[offset + GRID] * (1.0 - fx) * fy
            + lum[offset + GRID + 1] * fx * fy
    }
}

struct Dot {
    x: f32,
    y: f32,
    sample: Sample,
}

#[derive(Default)]
struct DonutCache {
    lum: Vec<f32>,
    orientation: Option<(u64, u64)>,
    width: Option<u32>,
    dots: Vec<Dot>,
}

impl DonutCache {
    fn prepare(&mut self, w: f32, tilt: f64, spin: f64) {
        let orientation = (tilt.to_bits(), spin.to_bits());
        if self.orientation != Some(orientation) {
            self.lum.resize(GRID * GRID, 0.0);
            // render clears the reused output, including pixels missed this frame.
            render(field_options(tilt, spin), &mut self.lum);
            self.orientation = Some(orientation);
        }
        if self.width == Some(w.to_bits()) {
            return;
        }
        self.dots.clear();
        let s = w / CELLS;
        let half = w / 2.0;
        let (sin_a, cos_a) = std::f32::consts::FRAC_PI_4.sin_cos();
        let ext = (CELLS * std::f32::consts::FRAC_1_SQRT_2).ceil() as i32 + 1;
        for j in -ext..=ext {
            for i in -ext..=ext {
                let (i, j) = (i as f32, j as f32);
                let x = half + (i * cos_a - j * sin_a) * s;
                let y = half + (i * sin_a + j * cos_a) * s;
                if x < -s || x > w + s || y < -s || y > w + s {
                    continue;
                }
                if let Some(sample) = Sample::new(x / w * GRID as f32, y / w * GRID as f32) {
                    self.dots.push(Dot { x, y, sample });
                }
            }
        }
        self.width = Some(w.to_bits());
    }
}

fn field_options(tilt: f64, spin: f64) -> DonutOptions {
    DonutOptions {
        columns: GRID,
        rows: GRID,
        tilt,
        spin,
        steps: 48,
        refine: true,
        sharp: false,
    }
}

#[derive(Default)]
struct Rotation {
    last: Option<Point<Pixels>>,
    tilt: f64,
    spin: f64,
    // The keyed rotation survives element recreation. Keep its renderer cache
    // alongside it, without notifying the view when only cached pixels change.
    cache: Rc<RefCell<DonutCache>>,
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
        if !side.is_finite()
            || side <= 0.
            || !f32::from(position.x).is_finite()
            || !f32::from(position.y).is_finite()
        {
            self.last = None;
            return;
        }
        if let Some(last) = self.last {
            let scale = std::f64::consts::TAU / side.max(1.) as f64;
            self.spin = angle(
                self.spin + (f32::from(position.x) as f64 - f32::from(last.x) as f64) * scale,
            );
            self.tilt = angle(
                self.tilt + (f32::from(position.y) as f64 - f32::from(last.y) as f64) * scale,
            );
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
    let cache = rotation.read(cx).cache.clone();
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
        .child(donut_canvas(side, ink, started, tilt, spin, cache))
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

fn donut_canvas(
    side: f32,
    ink: Hsla,
    started: Instant,
    tilt: f64,
    spin: f64,
    cache: Rc<RefCell<DonutCache>>,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
            let (tilt, spin, animate) = orientation(
                started.elapsed().as_secs_f64(),
                crate::system::reduce_motion(),
                tilt,
                spin,
            );
            let w = f32::from(bounds.size.width);
            let mut cache = cache.borrow_mut();
            cache.prepare(w, tilt, spin);
            let s = w / CELLS;
            for dot in &cache.dots {
                let (px_, py_) = (dot.x, dot.y);
                let v = dot.sample.read(&cache.lum);
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
            if animate {
                window.request_animation_frame();
            }
        },
    )
    .size(px(side))
    .flex_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original sampling/geometry path, intentionally independent of the cache.
    fn legacy_dots(lum: &[f32], w: f32) -> Vec<(u32, u32, u32)> {
        let s = w / CELLS;
        let half = w / 2.0;
        let (sin_a, cos_a) = std::f32::consts::FRAC_PI_4.sin_cos();
        let ext = (CELLS * std::f32::consts::FRAC_1_SQRT_2).ceil() as i32 + 1;
        let mut dots = Vec::new();
        for j in -ext..=ext {
            for i in -ext..=ext {
                let (i, j) = (i as f32, j as f32);
                let x = half + (i * cos_a - j * sin_a) * s;
                let y = half + (i * sin_a + j * cos_a) * s;
                if x < -s || x > w + s || y < -s || y > w + s {
                    continue;
                }
                let (gx, gy) = (x / w * GRID as f32, y / w * GRID as f32);
                let g = GRID as f32;
                let v = if gx < 0.0 || gy < 0.0 || gx > g - 2.0 || gy > g - 2.0 {
                    0.0
                } else {
                    let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
                    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
                    let at = |x: usize, y: usize| lum[x + y * GRID];
                    at(x0, y0) * (1.0 - fx) * (1.0 - fy)
                        + at(x0 + 1, y0) * fx * (1.0 - fy)
                        + at(x0, y0 + 1) * (1.0 - fx) * fy
                        + at(x0 + 1, y0 + 1) * fx * fy
                };
                if v <= 0.04 {
                    continue;
                }
                let r = v.powf(0.85) * s * 0.48;
                dots.push(((x - r).to_bits(), (y - r).to_bits(), r.to_bits()));
            }
        }
        dots
    }

    fn cached_dots(cache: &DonutCache, w: f32) -> Vec<(u32, u32, u32)> {
        let s = w / CELLS;
        cache
            .dots
            .iter()
            .filter_map(|dot| {
                let v = dot.sample.read(&cache.lum);
                if v <= 0.04 {
                    return None;
                }
                let r = v.powf(0.85) * s * 0.48;
                Some(((dot.x - r).to_bits(), (dot.y - r).to_bits(), r.to_bits()))
            })
            .collect()
    }

    #[test]
    fn cached_fields_and_calibrated_dot_geometry_match_legacy_bits() {
        let mut cache = DonutCache::default();
        for (tilt, spin) in [(1., 0.), (1.2, 0.7), (-0.3, 5.9), (1., 0.)] {
            let mut legacy = vec![0.; GRID * GRID];
            render(field_options(tilt, spin), &mut legacy);
            for w in [140., 139.75, 280., 73.125, 140.] {
                cache.prepare(w, tilt, spin);
                assert_eq!(cache.lum, legacy, "reused field must clear stale pixels");
                assert_eq!(cached_dots(&cache, w), legacy_dots(&legacy, w));
            }
        }
    }

    #[test]
    fn identical_orientation_reuses_field_and_grid_but_drag_invalidates_field() {
        let mut cache = DonutCache::default();
        cache.prepare(140., 1., 0.);
        let field = cache.lum.as_ptr();
        let grid = cache.dots.as_ptr();
        // A sentinel proves a cache hit skips rendering, rather than just producing
        // an equal field. Width changes must not invalidate the luminance field.
        cache.lum[0] = -123.;
        cache.prepare(140., 1., 0.);
        assert_eq!(cache.lum[0], -123.);
        assert_eq!(cache.dots.as_ptr(), grid);
        cache.prepare(141., 1., 0.);
        assert_eq!(cache.lum[0], -123.);
        cache.prepare(141., 1.5, 0.75);
        assert_eq!(cache.lum.as_ptr(), field);
        let mut expected = vec![0.; GRID * GRID];
        render(field_options(1.5, 0.75), &mut expected);
        assert_eq!(cache.lum, expected);
    }

    #[test]
    #[ignore = "CPU comparison benchmark: run with --ignored --nocapture"]
    fn benchmark_legacy_vs_cached_donut() {
        use std::hint::black_box;
        for reduced in [false, true] {
            let frames = 100;
            let mut cache = DonutCache::default();
            cache.prepare(140., 1., 0.); // Compare steady-state rerenders.
            let start = Instant::now();
            for frame in 1..=frames {
                let (tilt, spin, _) = orientation(frame as f64 / 60., reduced, 0., 0.);
                let mut lum = vec![0.; GRID * GRID];
                render(field_options(tilt, spin), &mut lum);
                black_box(legacy_dots(black_box(&lum), black_box(140.)));
            }
            let legacy = start.elapsed();
            let start = Instant::now();
            for frame in 1..=frames {
                let (tilt, spin, _) = orientation(frame as f64 / 60., reduced, 0., 0.);
                cache.prepare(black_box(140.), black_box(tilt), black_box(spin));
                black_box(cached_dots(black_box(&cache), black_box(140.)));
            }
            let optimized = start.elapsed();
            eprintln!(
                "donut reduced={reduced}, {frames} frames: legacy={legacy:?}, cached={optimized:?}, speedup={:.2}x; field allocations {frames} -> 0 (steady state)",
                legacy.as_secs_f64() / optimized.as_secs_f64()
            );
        }
    }

    #[test]
    fn reduced_motion_freezes_time_but_preserves_explicit_rotation() {
        assert_eq!(orientation(0., true, 0., 0.), (1., 0., false));
        for elapsed in [1., 1000., f64::MAX, f64::NAN, f64::INFINITY] {
            assert_eq!(orientation(elapsed, true, 0., 0.), (1., 0., false));
            assert_eq!(orientation(elapsed, true, 0.5, 0.75), (1.5, 0.75, false));
        }
        let mut rotation = Rotation {
            last: Some(point(px(0.), px(0.))),
            ..Default::default()
        };
        rotation.drag_to(point(px(35.), px(35.)), 140.);
        assert_eq!(
            orientation(5., true, rotation.tilt, rotation.spin),
            (
                1. + std::f64::consts::FRAC_PI_2,
                std::f64::consts::FRAC_PI_2,
                false
            )
        );
    }

    #[test]
    fn normal_motion_animates_with_bounded_finite_orientation() {
        assert_eq!(orientation(0., false, 0., 0.), (1., 0., true));
        let (tilt, spin, animate) = orientation(1., false, 0., 0.);
        assert!((tilt - (1. + 0.4f64.sin() * 0.25)).abs() < 1e-12);
        assert_eq!(spin, 0.7);
        assert!(animate);
        for elapsed in [-1., f64::NAN, f64::INFINITY, f64::MAX] {
            let (tilt, spin, _) = orientation(elapsed, false, f64::INFINITY, f64::NAN);
            assert!(tilt.is_finite() && spin.is_finite());
            assert!(tilt.abs() <= std::f64::consts::TAU);
            assert!(spin.abs() <= std::f64::consts::TAU);
        }
    }

    #[test]
    fn invalid_drag_input_cannot_poison_orientation() {
        for (position, side) in [
            (point(px(f32::NAN), px(0.)), 140.),
            (point(px(0.), px(f32::INFINITY)), 140.),
            (point(px(1.), px(1.)), f32::NAN),
            (point(px(1.), px(1.)), 0.),
        ] {
            let mut rotation = Rotation {
                last: Some(point(px(0.), px(0.))),
                tilt: 0.5,
                spin: 0.75,
                ..Default::default()
            };
            rotation.drag_to(position, side);
            assert_eq!((rotation.tilt, rotation.spin), (0.5, 0.75));
            assert!(rotation.last.is_none());
        }
    }

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
    fn rendered_donut_cancel_outside_release_and_competing_contact(cx: &mut gpui::TestAppContext) {
        use gpui::{PlatformInput, TouchEvent, TouchId, TouchPhase};
        let (view, cx) = cx.add_window_view(|_, cx| DonutFixture {
            rotation: cx.new(|_| Rotation::default()),
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let start = cx.debug_bounds("interactive-hero-donut").unwrap().center();
        let moved = start + point(px(20.), px(15.));
        let outside = start + point(px(300.), px(300.));
        let dispatch = |cx: &mut gpui::VisualTestContext, id, phase, position| {
            cx.update(|window, cx| {
                window.dispatch_event(
                    PlatformInput::Touch(TouchEvent {
                        id: TouchId(id),
                        phase,
                        position,
                        predicted_position: None,
                        force: None,
                    }),
                    cx,
                );
                window.draw(cx).clear(cx);
            });
        };
        for terminal in [TouchPhase::Cancelled, TouchPhase::Ended] {
            dispatch(cx, 1, TouchPhase::Started, start);
            dispatch(cx, 1, TouchPhase::Moved, moved);
            let before = view.read_with(cx, |view, cx| {
                let state = view.rotation.read(cx);
                (state.tilt, state.spin)
            });
            dispatch(cx, 2, TouchPhase::Started, start);
            dispatch(cx, 2, TouchPhase::Moved, outside);
            dispatch(cx, 2, TouchPhase::Ended, outside);
            view.read_with(cx, |view, cx| {
                let state = view.rotation.read(cx);
                assert_eq!(
                    (state.tilt, state.spin),
                    before,
                    "competing contact must not rotate"
                );
                assert!(
                    state.last.is_some(),
                    "competing release must not end owner drag"
                );
            });
            dispatch(cx, 1, terminal, outside);
            let released = view.read_with(cx, |view, cx| {
                let state = view.rotation.read(cx);
                assert!(state.last.is_none());
                if terminal == TouchPhase::Cancelled {
                    assert_eq!((state.tilt, state.spin), before);
                }
                (state.tilt, state.spin)
            });
            dispatch(cx, 1, TouchPhase::Moved, start);
            view.read_with(cx, |view, cx| {
                let state = view.rotation.read(cx);
                assert_eq!((state.tilt, state.spin), released);
            });
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
