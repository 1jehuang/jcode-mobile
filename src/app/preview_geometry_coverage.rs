//! Geometry used by the actual mask painter, not screenshot/GPU acceptance.
use super::*;

#[test]
fn preview_mask_corners_are_symmetric_bounded_and_follow_circle_geometry() {
    let mut random = 0x50524556494557_u64;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 7;
        random ^= random << 17;
        random
    };
    for case in 0..1024 {
        let width = 1. + (next() % 20000) as f32 / 10.;
        let height = 1. + (next() % 20000) as f32 / 10.;
        let ox = (next() % 2000) as f32 / 10. - 100.;
        let oy = (next() % 2000) as f32 / 10. - 100.;
        let bounds = gpui::Bounds::new(
            gpui::point(px(ox), px(oy)),
            gpui::size(px(width), px(height)),
        );
        let radius = 48_f64.min(width as f64 / 2.).min(height as f64 / 2.);
        let mut canonical = None;
        for (right, bottom) in [(false, false), (true, false), (false, true), (true, true)] {
            let points = preview_corner_polygon(bounds, right, bottom).unwrap();
            let local: Vec<(f64, f64)> = points
                .iter()
                .map(|p| {
                    let x = f32::from(p.x) - ox;
                    let y = f32::from(p.y) - oy;
                    assert!(
                        x >= -0.001 && x <= width + 0.001 && y >= -0.001 && y <= height + 0.001,
                        "case={case}"
                    );
                    (
                        if right {
                            width as f64 - x as f64
                        } else {
                            x as f64
                        },
                        if bottom {
                            height as f64 - y as f64
                        } else {
                            y as f64
                        },
                    )
                })
                .collect();
            assert!(local[0].0.abs() < 0.001 && local[0].1.abs() < 0.001);
            assert!((local[1].0 - radius).abs() < 0.001 && local[1].1.abs() < 0.001);
            for &(x, y) in &local[1..] {
                assert!(
                    ((x - radius).hypot(y - radius) - radius).abs() < 0.001,
                    "arc case={case}"
                );
            }
            let twice_area: f64 = local
                .iter()
                .zip(local.iter().cycle().skip(1))
                .take(local.len())
                .map(|(&(x, y), &(nx, ny))| x * ny - nx * y)
                .sum();
            let ideal = radius * radius * (1. - std::f64::consts::PI / 4.);
            assert!(
                (twice_area.abs() / 2. - ideal).abs() < radius * radius * 0.002 + 0.001,
                "mask area case={case}"
            );
            if let Some(expected) = &canonical {
                let expected: &Vec<(f64, f64)> = expected;
                for (&(x, y), &(a, b)) in local.iter().zip(expected) {
                    assert!(
                        (x - a).abs() < 0.001 && (y - b).abs() < 0.001,
                        "mirror case={case}"
                    );
                }
            } else {
                canonical = Some(local);
            }
        }
    }
}

#[test]
fn preview_mask_rejects_empty_and_nonfinite_bounds() {
    for (x, y, w, h) in [
        (0., 0., 0., 10.),
        (0., 0., 10., 0.),
        (0., 0., -1., 10.),
        (0., 0., f32::NAN, 10.),
        (0., 0., 10., f32::INFINITY),
        (f32::INFINITY, 0., 10., 10.),
        (0., f32::NAN, 10., 10.),
    ] {
        let bounds = gpui::Bounds::new(gpui::point(px(x), px(y)), gpui::size(px(w), px(h)));
        assert!(preview_corner_polygon(bounds, false, false).is_none());
    }
}
