//! Horizontal swipe between session tabs, modeled on the desktop workspace
//! pull, but restrained for a phone: vertical scrolling stays locked, panel
//! travel is capped, and deliberate swipes reveal the neighbor with a small nudge.

use std::time::{Duration, Instant};

/// Distance (px) of finger travel that counts as a committed swipe.
pub const COMMIT: f32 = 120.0;
/// Creating a session requires continuing beyond an ordinary tab swipe.
pub const NEW_SESSION_COMMIT: f32 = COMMIT * 2.0;
/// How long the snap-back takes after a canceled swipe.
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
    /// Destination panel's visual offset after navigation commits. Kept
    /// separate so raw rubber-band consumers retain their existing behavior.
    switch_settle: Option<(f32, Instant)>,
    /// `cancel` has no neighbor arguments. Reuse the last rendered geometry
    /// instead of turning an edge pull into a full-strength cancellation.
    panel_neighbors: Option<(bool, bool)>,
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
    /// The virtual panel after the last tab offers creation, never at the
    /// opposite edge or while vertical scrolling. Use the same content-motion
    /// convention as tab navigation: leftward travel reveals the panel right.
    pub fn new_session_preview(&self, can_next: bool) -> Option<f32> {
        (self.active && !can_next && self.dx < -COMMIT)
            .then(|| ((-self.dx - COMMIT) / (NEW_SESSION_COMMIT - COMMIT)).clamp(0.0, 1.0))
    }

    /// Destination direction and progress toward committing, only while dragging.
    /// Missing neighbors and vertical scrolling never advertise navigation.
    pub fn preview(&self, can_prev: bool, can_next: bool) -> Option<(i32, f32)> {
        if !self.active {
            return None;
        }
        let direction = if self.dx < 0.0 && can_next {
            1
        } else if self.dx > 0.0 && can_prev {
            -1
        } else {
            return None;
        };
        Some((direction, (self.dx.abs() / COMMIT).clamp(0.0, 1.0)))
    }

    /// Cancel recognition without ever committing navigation. Keep a bounded
    /// snap-back animation, but discard travel and direction lock so a late
    /// release (or the idle-end handler) cannot switch tabs.
    pub fn cancel(&mut self, width: f32, now: Instant) {
        let landing = self.switch_settle.is_some();
        let (can_prev, can_next) = self.panel_neighbors.unwrap_or((true, true));
        let from = if landing {
            self.panel_offset(width, can_prev, can_next, now).0
        } else {
            self.offset(width, can_prev, can_next, now).0
        };
        self.dx = 0.0;
        self.dy = 0.0;
        self.vertical = false;
        self.active = false;
        self.last_event = None;
        self.settle = (!landing && from != 0.0).then_some((from, now));
        self.switch_settle = (landing && from != 0.0).then_some((from, now));
    }

    /// Feeds a scroll delta. Returns true when the gesture is horizontal and was consumed.
    pub fn feed(&mut self, dx: f32, dy: f32, now: Instant) -> bool {
        if self
            .last_event
            .is_some_and(|last| now.duration_since(last) > IDLE_END)
        {
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
                self.switch_settle = None;
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
            self.settle = None;
            self.begin_switch_settle(dx, width, now);
            Release::Switch(1)
        } else if dx >= COMMIT && can_prev {
            self.settle = None;
            self.begin_switch_settle(dx, width, now);
            Release::Switch(-1)
        } else {
            self.switch_settle = None;
            let edge = (dx > 0.0 && !can_prev) || (dx < 0.0 && !can_next);
            self.settle = Some((resist(dx, width) * if edge { 0.25 } else { 1.0 }, now));
            Release::Cancel
        }
    }

    fn begin_switch_settle(&mut self, dx: f32, width: f32, now: Instant) {
        let outgoing = (resist(dx, width) * 6.0).clamp(-width * 0.8, width * 0.8);
        // The destination was adjacent to the outgoing panel before commit.
        // Preserve that exact position when it becomes the active panel.
        let incoming = -(width - outgoing.abs()).copysign(dx);
        self.switch_settle = Some((incoming, now));
    }

    /// Final visual offset, including neighbor amplification while dragging
    /// and a continuous destination landing after a committed tab switch.
    pub fn panel_offset(
        &mut self,
        width: f32,
        can_prev: bool,
        can_next: bool,
        now: Instant,
    ) -> (f32, bool) {
        self.panel_offset_with_motion(
            width,
            can_prev,
            can_next,
            now,
            crate::system::reduce_motion(),
        )
    }

    fn panel_offset_with_motion(
        &mut self,
        width: f32,
        can_prev: bool,
        can_next: bool,
        now: Instant,
        reduce_motion: bool,
    ) -> (f32, bool) {
        self.panel_neighbors = Some((can_prev, can_next));
        if let Some((from, start)) = self.switch_settle {
            let t = now.saturating_duration_since(start).as_secs_f32() / SETTLE.as_secs_f32();
            if t >= 1.0 || reduce_motion {
                self.switch_settle = None;
                return (0.0, false);
            }
            return (from * (1.0 - ease_out_cubic(t)), true);
        }
        if reduce_motion && !self.active {
            self.settle = None;
            return (0.0, false);
        }
        let (raw, moving) = self.offset(width, can_prev, can_next, now);
        let neighbor = (raw > 0.0 && can_prev) || (raw < 0.0 && can_next);
        (
            if neighbor {
                (raw * 6.0).clamp(-width * 0.8, width * 0.8)
            } else {
                raw
            },
            moving,
        )
    }

    /// Current horizontal offset of the chat panel, and whether it is still moving.
    pub fn offset(
        &mut self,
        width: f32,
        can_prev: bool,
        can_next: bool,
        now: Instant,
    ) -> (f32, bool) {
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

    #[test]
    fn committed_panel_lands_from_its_exact_neighbor_position_both_directions() {
        let now = Instant::now();
        for width in [240., 390., 768.] {
            for dx in [-COMMIT, COMMIT, -1000., 1000.] {
                let mut s = Swipe::default();
                s.feed(dx, 0., now);
                let (outgoing, _) = s.panel_offset_with_motion(width, true, true, now, false);
                let direction = if dx < 0. { 1 } else { -1 };
                let neighbor_before = outgoing + width * direction as f32;
                assert!(
                    matches!(s.release(width, true, true, now), Release::Switch(d) if d == direction)
                );
                // After switching, neighbor availability may be entirely different.
                let (incoming, moving) =
                    s.panel_offset_with_motion(width, false, false, now, false);
                assert!((incoming - neighbor_before).abs() < 0.001);
                assert!(moving);
                assert_eq!(incoming.signum(), direction as f32);
                let mut previous = incoming.abs();
                for ms in [1, 16, 40, 80, 159, 160, 200] {
                    let (offset, moving) = s.panel_offset_with_motion(
                        width,
                        false,
                        false,
                        now + Duration::from_millis(ms),
                        false,
                    );
                    assert!(offset.abs() <= previous);
                    assert!(
                        offset == 0. || offset.signum() == incoming.signum(),
                        "landing crossed to opposite direction"
                    );
                    if ms >= 160 {
                        assert_eq!((offset, moving), (0., false));
                    }
                    previous = offset.abs();
                }
            }
        }
    }

    #[test]
    fn canceled_panels_keep_visual_position_with_and_without_neighbors() {
        let now = Instant::now();
        for (dx, prev, next) in [
            (60., true, true),
            (-60., true, true),
            (200., false, true),
            (-200., true, false),
        ] {
            for explicit_cancel in [false, true] {
                let mut s = Swipe::default();
                s.feed(dx, 0., now);
                let before = s.panel_offset(390., prev, next, now).0;
                if explicit_cancel {
                    s.cancel(390., now);
                } else {
                    assert!(matches!(s.release(390., prev, next, now), Release::Cancel));
                }
                let (after, _) = s.panel_offset(390., prev, next, now);
                if crate::system::reduce_motion() {
                    assert_eq!(after, 0.);
                } else {
                    assert!((before - after).abs() < 0.001);
                }
                assert!(s.switch_settle.is_none());
                assert_eq!(s.panel_offset(390., prev, next, now + SETTLE), (0., false));
            }
        }
    }

    #[test]
    fn cancel_during_landing_keeps_position_and_new_gesture_discards_old_landing() {
        let now = Instant::now();
        for dx in [-COMMIT, COMMIT] {
            let mut s = Swipe::default();
            s.feed(dx, 0., now);
            s.release(390., true, true, now);
            let midway = now + Duration::from_millis(40);
            let before = s.panel_offset(390., true, true, midway).0;
            s.cancel(390., midway);
            assert_eq!(s.panel_offset(390., true, true, midway).0, before);
            assert!(matches!(
                s.release(390., true, true, midway),
                Release::Cancel
            ));
            assert_eq!(
                s.panel_offset(390., true, true, midway + SETTLE),
                (0., false)
            );
            s.feed(dx, 0., now + Duration::from_secs(1));
            s.release(390., true, true, now + Duration::from_secs(1));
            assert!(s.switch_settle.is_some());
            s.feed(-dx, 0., now + Duration::from_secs(1));
            assert!(s.switch_settle.is_none());
            assert_eq!(
                s.panel_offset(390., true, true, now + Duration::from_secs(1))
                    .0
                    .signum(),
                (-dx).signum()
            );
        }
    }

    #[test]
    fn reduced_motion_snaps_committed_and_canceled_panel_settlements() {
        let now = Instant::now();
        for dx in [-COMMIT, COMMIT, -60., 60.] {
            let mut s = Swipe::default();
            s.feed(dx, 0., now);
            s.release(390., true, true, now);
            assert_eq!(
                s.panel_offset_with_motion(390., true, true, now, true),
                (0., false)
            );
            assert!(s.switch_settle.is_none() && s.settle.is_none());
            assert_eq!(
                s.panel_offset_with_motion(390., true, true, now, false),
                (0., false)
            );
        }
    }

    #[test]
    fn new_session_requires_continued_travel_at_next_edge() {
        let now = Instant::now();
        let mut s = Swipe::default();
        s.feed(-COMMIT, 0., now);
        assert_eq!(s.new_session_preview(false), None);
        s.feed(-60., 0., now);
        assert_eq!(s.new_session_preview(false), Some(0.5));
        assert_eq!(s.new_session_preview(true), None);
        s.feed(-60., 0., now);
        assert_eq!(s.new_session_preview(false), Some(1.));
        s.cancel(390., now);
        assert_eq!(s.new_session_preview(false), None);
        s.feed(NEW_SESSION_COMMIT, 0., now);
        assert_eq!(s.new_session_preview(false), None);
        s.release(390., false, false, now);
        s.feed(0., 30., now);
        s.feed(-NEW_SESSION_COMMIT, 0., now);
        assert_eq!(s.new_session_preview(false), None);
    }

    #[test]
    fn tab_preview_tracks_direction_threshold_and_cancellation() {
        let now = Instant::now();
        let mut s = Swipe::default();
        assert_eq!(s.preview(true, true), None);
        s.feed(-60., 0., now);
        assert_eq!(s.preview(true, true), Some((1, 0.5)));
        assert_eq!(s.preview(true, false), None);
        s.feed(-180., 0., now);
        assert_eq!(s.preview(true, true), Some((1, 1.)));
        s.feed(300., 0., now);
        assert_eq!(s.preview(true, true), Some((-1, 0.5)));
        assert_eq!(s.preview(false, true), None);
        s.cancel(390., now);
        assert_eq!(s.preview(true, true), None);
        s.feed(0., 30., now);
        assert_eq!(s.preview(true, true), None);
    }

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
                        if s.last_event
                            .is_some_and(|last| now.duration_since(last) > IDLE_END)
                        {
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
                            Release::Switch(-1) => {
                                assert!(active && !locked && prev && dx >= COMMIT)
                            }
                            Release::Switch(1) => {
                                assert!(active && !locked && next_tab && dx <= -COMMIT)
                            }
                            Release::Switch(other) => panic!("invalid direction {other}"),
                            Release::Cancel => assert!(
                                !active
                                    || locked
                                    || dx.abs() < COMMIT
                                    || (dx > 0. && !prev)
                                    || (dx < 0. && !next_tab)
                            ),
                        }
                        assert!(matches!(
                            s.release(width, prev, next_tab, now),
                            Release::Cancel
                        ));
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
                assert!(
                    offset.is_finite() && offset.abs() <= 48.,
                    "seed={initial_seed} step={step}"
                );
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
            assert!(matches!(
                s.release(390., true, true, now + IDLE_END),
                Release::Cancel
            ));
            assert_eq!(s.offset(390., true, true, now + SETTLE), (0., false));
            now += Duration::from_secs(1);
            assert!(
                s.feed(-COMMIT, 0., now),
                "cancellation must allow a fresh gesture"
            );
            assert!(matches!(
                s.release(390., true, true, now),
                Release::Switch(1)
            ));
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
            assert!(matches!(
                s.release(390., false, false, now),
                Release::Cancel
            ));
            let mut previous = f32::INFINITY;
            for ms in [0, 1, 16, 40, 80, 159, 160, 200] {
                let (offset, moving) =
                    s.offset(390., false, false, now + Duration::from_millis(ms));
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
        assert!(matches!(
            s.release(390.0, true, true, now),
            Release::Switch(1)
        ));
        // A committed switch must not start a second panel animation.
        assert_eq!(s.offset(390.0, true, true, now), (0.0, false));
        let (o, moving) = s.offset(390.0, true, true, now + SETTLE);
        assert!(!moving && o == 0.0);
    }

    #[test]
    fn committed_swipes_in_either_direction_have_no_followup_motion() {
        let now = Instant::now();
        for (dx, direction) in [(-COMMIT, 1), (COMMIT, -1)] {
            let mut s = Swipe::default();
            assert!(s.feed(dx, 0., now));
            assert!(s.offset(390., true, true, now).0.abs() > 0.);
            assert!(
                matches!(s.release(390., true, true, now), Release::Switch(d) if d == direction)
            );
            for ms in [0, 16, 80, 160, 200] {
                assert_eq!(
                    s.offset(390., true, true, now + Duration::from_millis(ms)),
                    (0., false)
                );
            }
        }
    }

    #[test]
    fn short_drag_or_missing_neighbor_snaps_back() {
        let mut s = Swipe::default();
        let now = Instant::now();
        s.feed(40.0, 0.0, now);
        assert!(matches!(s.release(390.0, true, true, now), Release::Cancel));
        s.feed(200.0, 0.0, now);
        assert!(matches!(
            s.release(390.0, false, true, now),
            Release::Cancel
        ));
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
        assert!(matches!(
            s.release(390.0, true, true, now + IDLE_END),
            Release::Switch(1)
        ));
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
