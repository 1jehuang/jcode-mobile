//! Frame-rate-independent following for a transcript pinned to its newest content.

/// Advance a positive scroll offset toward the newest content (`target`).
///
/// Large history/layout jumps and reduced motion snap immediately. Small streamed
/// changes ease at 14/s, with bounded frame time so a stalled frame cannot cause
/// a sudden catch-up. The result never escapes the current scrollable range.
pub fn offset(current: f32, target: f32, viewport: f32, dt: f32, reduce_motion: bool) -> f32 {
    let target = if target.is_finite() {
        target.max(0.0)
    } else {
        0.0
    };
    let current = if current.is_finite() {
        current.clamp(0.0, target)
    } else {
        target
    };
    let gap = target - current;
    if reduce_motion || gap > viewport.max(0.0) || gap <= 0.5 {
        return target;
    }
    let dt = if dt.is_finite() {
        dt.clamp(0.0, 0.05)
    } else {
        0.0
    };
    let next = current + gap * (1.0 - (-14.0 * dt).exp());
    if target - next <= 0.5 {
        target
    } else {
        next.clamp(0.0, target)
    }
}

#[cfg(test)]
mod tests {
    use super::offset;

    #[test]
    fn streamed_growth_is_frame_rate_independent() {
        fn simulate(frames_per_step: usize) -> f32 {
            let mut current = 0.0;
            for step in 1..=10 {
                let target = step as f32 * 10.0;
                for _ in 0..frames_per_step {
                    current = offset(current, target, 600.0, 0.1 / frames_per_step as f32, false);
                    assert!((0.0..=target).contains(&current));
                }
            }
            current
        }
        let at_60_hz = simulate(6);
        let at_120_hz = simulate(12);
        assert!((at_60_hz - at_120_hz).abs() < 0.001);
        assert!(at_60_hz > 95.0 && at_60_hz < 100.0);
    }

    #[test]
    fn history_jumps_snap_but_one_viewport_still_eases() {
        assert_eq!(offset(100.0, 701.0, 600.0, 1.0 / 60.0, false), 701.0);
        let eased = offset(100.0, 700.0, 600.0, 1.0 / 60.0, false);
        assert!(eased > 100.0 && eased < 700.0);
    }

    #[test]
    fn reduced_motion_snaps_even_without_elapsed_time() {
        assert_eq!(offset(100.0, 120.0, 600.0, 0.0, true), 120.0);
    }

    #[test]
    fn shrinking_content_and_invalid_offsets_are_clamped() {
        assert_eq!(offset(800.0, 200.0, 600.0, 0.016, false), 200.0);
        assert_eq!(offset(200.0, -10.0, 600.0, 0.016, false), 0.0);
        assert_eq!(offset(-10.0, 100.0, 600.0, 0.0, false), 0.0);
        assert_eq!(offset(f32::NAN, 100.0, 600.0, 0.016, false), 100.0);
        assert_eq!(offset(100.0, f32::NAN, 600.0, 0.016, false), 0.0);
    }

    #[test]
    fn frame_time_is_bounded_and_never_overshoots() {
        assert_eq!(offset(10.0, 100.0, 600.0, -1.0, false), 10.0);
        assert_eq!(offset(10.0, 100.0, 600.0, 0.0, false), 10.0);
        assert_eq!(
            offset(10.0, 100.0, 600.0, 1.0, false),
            offset(10.0, 100.0, 600.0, 0.05, false)
        );
        assert_eq!(offset(99.5, 100.0, 600.0, 0.0, false), 100.0);
        let mut current = 0.0;
        for _ in 0..100 {
            current = offset(current, 100.0, 600.0, 0.05, false);
            assert!((0.0..=100.0).contains(&current));
        }
        assert_eq!(current, 100.0);
    }
}
