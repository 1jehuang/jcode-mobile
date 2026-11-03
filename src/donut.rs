//! The jcode hero donut, ported 1:1 from jcode.sh (public/donut.js): the
//! website's own `site-visual` torus field, screened through the 45-degree
//! halftone lattice with the same gamma, radius and motion.

use std::time::Instant;

use gpui::{App, Bounds, Hsla, Pixels, Window, canvas, point, prelude::*, px, size};
use crate::donut_field::{DonutOptions, render};

/// Luminance field resolution (site: 96 to 152 adaptive).
const GRID: usize = 96;
/// Halftone screen density across the canvas (the brand mark uses 50).
const CELLS: f32 = 50.0;

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

pub fn spinning_donut(side: f32, ink: Hsla, started: Instant) -> impl IntoElement {
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
                    tilt: 1.0 + (t * 0.4).sin() * 0.25,
                    spin: t * 0.7,
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
            let ext = (CELLS * 0.7071).ceil() as i32 + 1;
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
                    let r = v.powf(0.85) * s * 0.62;
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
