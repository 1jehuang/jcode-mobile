//! Horizontal swipe between session tabs, modeled on the desktop workspace
//! pull, but restrained for a phone: vertical scrolling stays locked, panel
//! travel is capped, and deliberate swipes reveal the neighbor with a small nudge.

use std::time::{Duration, Instant};

/// Distance (px) of finger travel that counts as a committed swipe.
pub const COMMIT: f32 = 120.0;
/// How long the slide-in takes after a commit, or the snap-back after a cancel.
pub const SETTLE: Duration = Duration::from_millis(160);
/// No wheel/touch event for this long ends the gesture (trackpads without phases).
pub const IDLE_END: Duration = Duration::from_millis(160);

#[derive(Default)]
pub struct Swipe {
    /// Raw horizontal travel of the current gesture.
    pub dx: f32,
    /// Vertical travel, so mostly-vertical scrolls never become swipes.
    dy: f32,
    /// Once scrolling vertically, keep that decision until the gesture ends.
    vertical: bool,
    pub active: bool,
    pub last_event: Option<Instant>,
    /// Release animation: (start offset, start time).
    settle: Option<(f32, Instant)>,
}

/// Rubber-band: direct at first, then increasingly resisted, like the desktop pull.
pub fn resist(dx: f32, width: f32) -> f32 {
    let limit = (width * 0.12).clamp(1.0, 48.0);
    let a = dx.abs();
    let r = limit * (1.0 - (-a / limit).exp());
    r.copysign(dx)
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub enum Release {
    /// Go to the previous (-1) or next (+1) tab.
    Switch(i32),
    Cancel,
}

impl Swipe {
    /// Cancel recognition without ever committing navigation. Keep a bounded
    /// snap-back animation, but discard travel and direction lock so a late
    /// release (or the idle-end handler) cannot switch tabs.
    pub fn cancel(&mut self, width: f32, now: Instant) {
        let from = self.offset(width, true, true, now).0;
        self.dx = 0.0;
        self.dy = 0.0;
        self.vertical = false;
        self.active = false;
        self.last_event = None;
        self.settle = (from != 0.0).then_some((from, now));
    }

    /// Feeds a scroll delta. Returns true when the gesture is horizontal and was consumed.
    pub fn feed(&mut self, dx: f32, dy: f32, now: Instant) -> bool {
        if self.last_event.is_some_and(|last| now.duration_since(last) > IDLE_END) {
            self.dx = 0.0;
            self.dy = 0.0;
            self.active = false;
            self.vertical = false;
        }
        self.last_event = Some(now);
        if self.vertical {
            return false;
        }
        if !self.active {
            self.dx += dx;
            self.dy += dy;
            if self.dx.abs() > 24.0 && self.dx.abs() > self.dy.abs() * 2.0 {
                self.active = true;
                self.settle = None;
            } else if self.dy.abs() > 12.0 {
                // Never reinterpret diagonal drift or momentum as navigation.
                self.vertical = true;
                self.dx = 0.0;
                self.dy = 0.0;
            }
            return self.active;
        }
        self.dx += dx;
        true
    }

    /// Ends the gesture. `can_prev`/`can_next` say whether a neighbor exists.
    pub fn release(&mut self, width: f32, can_prev: bool, can_next: bool, now: Instant) -> Release {
        let dx = self.dx;
        let was = self.active;
        self.active = false;
        self.dx = 0.0;
        self.dy = 0.0;
        self.vertical = false;
        self.last_event = None;
        if !was {
            return Release::Cancel;
        }
        // Content moves with the finger: dragging left (dx < 0) reveals the next tab.
        if dx <= -COMMIT && can_next {
            self.settle = Some((resist(width, width), now));
            Release::Switch(1)
        } else if dx >= COMMIT && can_prev {
            self.settle = Some((resist(-width, width), now));
            Release::Switch(-1)
        } else {
            let edge = (dx > 0.0 && !can_prev) || (dx < 0.0 && !can_next);
            self.settle = Some((resist(dx, width) * if edge { 0.25 } else { 1.0 }, now));
            Release::Cancel
        }
    }

    /// Current horizontal offset of the chat panel, and whether it is still moving.
    pub fn offset(&mut self, width: f32, can_prev: bool, can_next: bool, now: Instant) -> (f32, bool) {
        if self.active {
            // Edges with no neighbor resist much harder.
            let edge = (self.dx > 0.0 && !can_prev) || (self.dx < 0.0 && !can_next);
            let r = resist(self.dx, width);
            return (if edge { r * 0.25 } else { r }, false);
        }
        if let Some((from, start)) = self.settle {
            let t = now.duration_since(start).as_secs_f32() / SETTLE.as_secs_f32();
            if t >= 1.0 || crate::system::reduce_motion() {
                self.settle = None;
                return (0.0, false);
            }
            return (from * (1.0 - ease_out_cubic(t)), true);
        }
        (0.0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixed PRNG, intentionally dependency-free and reproducible across platforms.
    fn next(seed: &mut u64) -> u32 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (*seed >> 32) as u32
    }

    #[test]
    fn seeded_swipe_feed_release_offset_state_machine() {
        for initial_seed in [1, 7, 42, 0xdead_beef, u64::MAX] {
            let mut seed = initial_seed;
            let mut s = Swipe::default();
            let mut now = Instant::now();
            let mut locked = false;
            for step in 0..4000 {
                let elapsed = next(&mut seed) % 220;
                now += Duration::from_millis(elapsed as u64);
                let width = [1., 240., 390., 768., 1440.][next(&mut seed) as usize % 5];
                let prev = next(&mut seed) & 1 != 0;
                let next_tab = next(&mut seed) & 1 != 0;
                match next(&mut seed) % 6 {
                    0..=2 => {
                        if s.last_event.is_some_and(|last| now.duration_since(last) > IDLE_END) {
                            locked = false;
                        }
                        let dx = (next(&mut seed) % 801) as f32 - 400.;
                        let dy = (next(&mut seed) % 501) as f32 - 250.;
                        let consumed = s.feed(dx, dy, now);
                        if locked {
                            assert!(!consumed && !s.active, "seed={initial_seed} step={step}");
                        }
                        locked |= s.vertical;
                    }
                    3 => {
                        let dx = s.dx;
                        let active = s.active;
                        match s.release(width, prev, next_tab, now) {
                            Release::Switch(-1) => assert!(active && !locked && prev && dx >= COMMIT),
                            Release::Switch(1) => assert!(active && !locked && next_tab && dx <= -COMMIT),
                            Release::Switch(other) => panic!("invalid direction {other}"),
                            Release::Cancel => assert!(!active || locked || dx.abs() < COMMIT
                                || (dx > 0. && !prev) || (dx < 0. && !next_tab)),
                        }
                        assert!(matches!(s.release(width, prev, next_tab, now), Release::Cancel));
                        locked = false;
                    }
                    4 => {
                        s.cancel(width, now);
                        assert!(!s.active && s.last_event.is_none());
                        assert_eq!((s.dx, s.dy), (0., 0.));
                        assert!(matches!(s.release(width, true, true, now), Release::Cancel));
                        locked = false;
                    }
                    _ => {}
                }
                let (offset, _) = s.offset(width, prev, next_tab, now);
                assert!(offset.is_finite() && offset.abs() <= 48., "seed={initial_seed} step={step}");
                assert!(s.dx.is_finite() && s.dy.is_finite());
                if !s.active && s.settle.is_none() {
                    assert_eq!(offset, 0.);
                }
            }
            s.release(390., false, false, now);
            assert_eq!(s.offset(390., false, false, now + SETTLE), (0., false));
        }
    }

    #[test]
    fn seeded_vertical_lock_never_drifts_into_navigation() {
        let mut seed = 12345;
        let mut now = Instant::now();
        for _ in 0..128 {
            let mut s = Swipe::default();
            assert!(!s.feed(0., 13., now));
            for _ in 0..64 {
                now += Duration::from_millis((next(&mut seed) % 150) as u64);
                assert!(!s.feed((next(&mut seed) % 4001) as f32 - 2000., 0., now));
                assert_eq!(s.offset(390., true, true, now), (0., false));
            }
            assert!(matches!(s.release(390., true, true, now), Release::Cancel));
        }
    }

    #[test]
    fn cancelled_committable_swipe_never_switches_on_late_release() {
        let mut seed = 57489;
        let mut now = Instant::now();
        for _ in 0..256 {
            let mut s = Swipe::default();
            let dx = COMMIT + (next(&mut seed) % 3000) as f32;
            s.feed(if next(&mut seed) & 1 == 0 { dx } else { -dx }, 0., now);
            assert!(s.active);
            s.cancel(390., now);
            assert!(!s.active && !s.vertical && s.last_event.is_none());
            assert_eq!((s.dx, s.dy), (0., 0.));
            assert!(matches!(s.release(390., true, true, now + IDLE_END), Release::Cancel));
            assert_eq!(s.offset(390., true, true, now + SETTLE), (0., false));
            now += Duration::from_secs(1);
            assert!(s.feed(-COMMIT, 0., now), "cancellation must allow a fresh gesture");
            assert!(matches!(s.release(390., true, true, now), Release::Switch(1)));
        }
    }

    #[test]
    fn seeded_absent_neighbors_never_switch_and_settle_is_monotonic() {
        let mut seed = 9876;
        let mut now = Instant::now();
        for _ in 0..256 {
            let mut s = Swipe::default();
            let dx = (next(&mut seed) % 8001) as f32 - 4000.;
            s.feed(dx, 0., now);
            assert!(matches!(s.release(390., false, false, now), Release::Cancel));
            let mut previous = f32::INFINITY;
            for ms in [0, 1, 16, 40, 80, 159, 160, 200] {
                let (offset, moving) = s.offset(390., false, false, now + Duration::from_millis(ms));
                assert!(offset.is_finite() && offset.abs() <= 12.);
                assert!(offset.abs() <= previous);
                previous = offset.abs();
                if ms >= 160 {
                    assert_eq!((offset, moving), (0., false));
                }
            }
            now += Duration::from_secs(1);
        }
    }

    #[test]
    fn vertical_scroll_is_not_a_swipe() {
        let mut s = Swipe::default();
        let now = Instant::now();
        assert!(!s.feed(2.0, 30.0, now));
        assert!(!s.feed(3.0, 20.0, now));
        assert!(!s.active);
    }

    #[test]
    fn far_left_drag_switches_to_next() {
        let mut s = Swipe::default();
        let now = Instant::now();
        assert!(s.feed(-30.0, 0.0, now));
        s.feed(-100.0, 0.0, now);
        assert!(matches!(s.release(390.0, true, true, now), Release::Switch(1)));
        // New panel enters subtly rather than crossing the whole screen.
        let (o, moving) = s.offset(390.0, true, true, now);
        if crate::system::reduce_motion() {
            assert_eq!((o, moving), (0.0, false));
        } else {
            assert!(moving && o > 0.0 && o <= 48.0);
        }
        let (o, moving) = s.offset(390.0, true, true, now + SETTLE);
        assert!(!moving && o == 0.0);
    }

    #[test]
    fn short_drag_or_missing_neighbor_snaps_back() {
        let mut s = Swipe::default();
        let now = Instant::now();
        s.feed(40.0, 0.0, now);
        assert!(matches!(s.release(390.0, true, true, now), Release::Cancel));
        s.feed(200.0, 0.0, now);
        assert!(matches!(s.release(390.0, false, true, now), Release::Cancel));
    }

    #[test]
    fn resistance_is_bounded() {
        assert!(resist(10_000.0, 390.0) <= 48.0);
        assert!((resist(5.0, 390.0) - 5.0).abs() < 0.3);
    }

    #[test]
    fn incidental_horizontal_motion_does_not_switch_sessions() {
        let now = Instant::now();
        let mut s = Swipe::default();
        assert!(!s.feed(18.0, 1.0, now));
        s.feed(80.0, 0.0, now);
        assert!(matches!(s.release(390.0, true, true, now), Release::Cancel));
    }

    #[test]
    fn vertical_gesture_stays_locked_despite_horizontal_drift() {
        let mut s = Swipe::default();
        let now = Instant::now();
        assert!(!s.feed(2.0, 30.0, now));
        assert!(!s.feed(180.0, 1.0, now));
        assert!(matches!(s.release(390.0, true, true, now), Release::Cancel));
        assert!(s.feed(-130.0, 0.0, now));
    }

    #[test]
    fn idle_resets_direction_and_uncommitted_travel() {
        let mut s = Swipe::default();
        let now = Instant::now();
        s.feed(0.0, 30.0, now);
        assert!(s.feed(-130.0, 0.0, now + IDLE_END + Duration::from_millis(1)));
        assert!(matches!(s.release(390.0, true, true, now + IDLE_END), Release::Switch(1)));
    }

    #[test]
    fn missing_neighbor_does_not_jump_on_release() {
        let mut s = Swipe::default();
        let now = Instant::now();
        s.feed(200.0, 0.0, now);
        let before = s.offset(390.0, false, true, now).0;
        s.release(390.0, false, true, now);
        let after = s.offset(390.0, false, true, now).0;
        if crate::system::reduce_motion() {
            // Accessibility removes the settling animation immediately.
            assert_eq!(after, 0.0);
        } else {
            assert_eq!(before, after);
        }
    }
}
